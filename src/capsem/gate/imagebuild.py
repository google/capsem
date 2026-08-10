"""Build profile-owned VM assets through the one config-owned image rail."""

from __future__ import annotations

from . import crossexec, initrd
from .actions import Call, Run
from .command import GateCommand
from .config import Arch, GateConfig
from .errors import GateError
from .execution import Step, step
from .imagebases import MaterializeRustBuilders, Prefetch, required_rust_builder_names
from .opacity import CallJustification, OpaqueKind
from .plan import Plan


def profiles(config: GateConfig) -> list[str]:
    """Every checked-in profile, by directory name."""
    found = sorted(path.parent.name for path in config.root.glob(config.imagebuild.profiles_glob))
    if not found:
        raise GateError(f"no profiles under {config.imagebuild.profiles_glob}")
    return found


def missing(config: GateConfig, arch: Arch) -> list[str]:
    """Which required artifacts this architecture's tree does not have.

    Present *and* non-empty. `is_file()` alone accepted a zero-length
    `vmlinuz`, which is exactly what a build that ran out of disk leaves --
    and this is the check meant to notice.
    """
    tree = config.path(config.imagebuild.output) / arch.name
    return [
        name
        for name in config.artifacts.bootable
        if not (tree / name).is_file() or (tree / name).stat().st_size == 0
    ]


def doctor(config: GateConfig) -> Step:
    """Check the host, with the asset and KVM checks turned off.

    Those two would fail on exactly the thing this is about to build, which is
    why the skips exist rather than the doctor being skipped entirely.
    """
    from . import doctor as diagnosis

    return step(
        "doctor",
        # Both halves of `just doctor`, composed rather than dispatched: the
        # gate's own wiring check, then the host-tooling script that actually
        # reads the skip variables.
        Call(
            "would the gate work if we started now",
            diagnosis.report,
            justification=CallJustification(
                kind=OpaqueKind.PURE_INSPECTION,
                reason="reports every wiring problem it can find and changes nothing at all",
                effects=frozenset({"process"}),
            ),
        ),
        Run(
            ["bash", config.doctor.common_script],
            env=dict(config.imagebuild.doctor_skips),
        ),
    )


def build_argv(
    config: GateConfig,
    *,
    profile: str,
    arch: str | None,
    template: str,
    output: str | None = None,
) -> list[str]:
    """The one spelling of `capsem-admin image build`.

    `output` defaults to the configured assets tree and is overridable because
    the concurrent asset lanes each need their own. It used to be a `just`
    parameter that the recipe accepted and never forwarded, so every lane wrote
    into the one shared directory while each checked a private one -- two
    architectures overwriting each other, and nothing looking at the result.
    """
    settings = config.imagebuild
    if template not in settings.templates:
        raise GateError(
            f"unknown image template {template!r}; expected one of {', '.join(settings.templates)}"
        )

    argv = [
        *settings.admin,
        "--profile",
        settings.profile_manifest.format(profile=profile),
        "--config-root",
        settings.config_root,
        "--output",
        output or settings.output,
        "--template",
        template,
        "--clean",
    ]
    if arch:
        argv += ["--arch", config.arch(arch).name]
    return argv


def build(
    config: GateConfig,
    *,
    profile: str,
    arch: str | None,
    template: str,
    output: str | None = None,
) -> Step:
    """One image build. The template is the only thing that varies."""
    label = f"image.{profile}.{template}" + (f".{arch}" if arch else "")
    return step(
        label,
        Run(build_argv(config, profile=profile, arch=arch, template=template, output=output)),
        contends=(config.exclusive("docker_daemon"),),
    )


class BuildAssetsCommand(
    GateCommand,
    name="build-assets",
    help="build one profile's VM assets, or every profile's",
):
    exclusive = True

    @classmethod
    def add_arguments(cls, parser) -> None:
        parser.add_argument("profile", nargs="?", help="defaults to every profile")
        parser.add_argument("arch", nargs="?", help="defaults to every architecture")
        parser.add_argument("--template", default="all", help="kernel, rootfs, or all")

    def plan(self) -> Plan:
        plan = Plan(self.name)
        config = self._config
        wanted = [self._args.profile] if self._args.profile else profiles(config)
        names = (
            (config.arch(self._args.arch).name,) if self._args.arch else tuple(config.architectures)
        )
        rust_builders = (
            () if self._args.template == "kernel" else required_rust_builder_names(config, names)
        )

        bases = plan.add(
            step(
                "base-images",
                Prefetch(names, rust_names=rust_builders),
                contends=(config.exclusive("docker_daemon"),),
            )
        )
        checked = plan.add(doctor(config), after=(bases,))
        ready = plan.add(
            step(
                "guest-execution",
                crossexec.Require(names),
                contends=(config.exclusive("docker_daemon"),),
            ),
            after=(checked,),
        )
        if rust_builders:
            ready = plan.add(
                step(
                    "guest-builders",
                    MaterializeRustBuilders(rust_builders),
                    contends=(config.exclusive("docker_daemon"),),
                ),
                after=(ready,),
            )
        images = tuple(
            plan.add(
                build(
                    config,
                    profile=profile,
                    arch=self._args.arch,
                    template=self._args.template,
                ),
                after=(ready,),
            )
            for profile in wanted
        )
        if self._args.template != "kernel":
            assets = config.path(config.imagebuild.output)
            targets = {name: (assets / name / config.artifacts.initrd,) for name in names}
            packed = plan.add(initrd.repack_step(config, targets), after=images)
            initrd.finalize(plan, config, assets=assets, after=(packed,))
        return plan


class CheckAssetsCommand(
    GateCommand,
    name="check-assets",
    help="build this host's VM assets if they are not already there",
):
    """The gate's precondition, using config's one architecture mapping."""

    exclusive = True

    def plan(self) -> Plan:
        plan = Plan(self.name)
        check_assets(plan, self._config)
        return plan


def check_assets(
    plan: Plan, config: GateConfig, *, after: tuple[Step, ...] = ()
) -> tuple[Step, ...]:
    """Build this host's VM assets if they are not already there.

    Returns the steps a caller should wait for, which is empty when the assets
    are already present -- so composing this into a larger plan sequences the
    next phase behind whatever actually ran, and behind nothing when nothing did.
    """
    arch = config.host_arch()
    if not missing(config, arch):
        return after

    phase = plan.phase("assets")
    names = (arch.name,)
    rust_builders = required_rust_builder_names(config, names)
    bases = phase.add(
        step(
            "base-images",
            Prefetch(names, rust_names=rust_builders),
            contends=(config.exclusive("docker_daemon"),),
        ),
        after=after,
    )
    checked = phase.add(doctor(config), after=(bases,))
    ready = phase.add(
        step(
            "guest-execution",
            crossexec.Require(names),
            contends=(config.exclusive("docker_daemon"),),
        ),
        after=(checked,),
    )
    if rust_builders:
        ready = phase.add(
            step(
                "guest-builders",
                MaterializeRustBuilders(rust_builders),
                contends=(config.exclusive("docker_daemon"),),
            ),
            after=(ready,),
        )
    return tuple(
        phase.add(
            build(config, profile=profile, arch=arch.name, template="all"),
            after=(ready,),
        )
        for profile in profiles(config)
    )


class ToolchainCommand(
    GateCommand,
    name="install-tools",
    help="install the cross-compilation targets and cargo tools a gate needs",
):
    """Idempotent: present means nothing happens, and nothing is said."""

    exclusive = True

    def plan(self) -> Plan:
        from . import toolchain

        plan = Plan(self.name)
        python = plan.add(toolchain.sync(self._config))
        plan.add(toolchain.rust(self._config), after=(python,))
        plan.add(toolchain.node(self._config), after=(python,))
        return plan


class NodeCommand(
    GateCommand,
    name="install-node",
    help="install every Node workspace a local gate exercises",
):
    """CI has separate jobs for docs, site and release-site. A local gate
    builds all of them in one checkout, so all of them are installed here."""

    exclusive = True

    def plan(self) -> Plan:
        from . import toolchain

        plan = Plan(self.name)
        plan.add(toolchain.node(self._config))
        return plan
