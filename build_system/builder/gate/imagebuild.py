"""Build profile-owned VM assets through the one config-owned image rail."""

from __future__ import annotations

from dataclasses import replace

from . import assetdependencies, crossexec, imagebases, initrd
from .actions import Run
from .assetcondition import AssetRecovery
from .assetcondition import missing as missing
from .command import GateCommand
from .config import GateConfig
from .errors import GateError
from .execution import Kind, Needs, Speed, Step, step
from .fileactions import Remove
from .imagedoctor import doctor
from .plan import Plan

#: Naming the capability sets rather than the whole declaration: `**dict`
#: unpacking would collapse three lines into one and take the type checking
#: with it, which is the thing these attributes exist to keep.
BUILDS = frozenset({Needs.DOCKER, Needs.DISK})
PULLS = frozenset({Needs.DOCKER, Needs.NETWORK})


def profiles(config: GateConfig) -> list[str]:
    """Every checked-in profile, by directory name."""
    found = sorted(path.parent.name for path in config.root.glob(config.imagebuild.profiles_glob))
    if not found:
        raise GateError(f"no profiles under {config.imagebuild.profiles_glob}")
    return found


def build_argv(
    config: GateConfig,
    *,
    profile: str,
    arch: str | None,
    template: str,
    output: str | None = None,
) -> list[str]:
    """The one spelling of `capsem-admin image build` for either output rail."""
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
        kind=Kind.PACKAGE, needs=BUILDS, speed=Speed.SLOW,
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
            ()
            if self._args.template == "kernel"
            else imagebases.required_rust_builder_names(config, names)
        )
        needs_asset_tools = self._args.template != "kernel"
        bases = plan.add(
            step(
                "base-images",
                imagebases.Prefetch(names, rust_names=rust_builders, asset_tools=needs_asset_tools),
                contends=(config.exclusive("docker_daemon"),),
        kind=Kind.PACKAGE, needs=PULLS, speed=Speed.SLOW,
            )
        )
        checked = plan.add(doctor(config), after=(bases,))
        ready = plan.add(
            step(
                "guest-execution",
                crossexec.Require(names),
                contends=(config.exclusive("docker_daemon"),),
        kind=Kind.PACKAGE, needs=BUILDS, speed=Speed.SLOW,
            ),
            after=(checked,),
        )
        if rust_builders:
            ready = plan.add(
                step(
                    "guest-builders",
                    imagebases.MaterializeRustBuilders(rust_builders),
                    contends=(config.exclusive("docker_daemon"),),
                    carry_checks=(imagebases.RequireRustBuilders(rust_builders),),
        kind=Kind.PACKAGE, needs=BUILDS, speed=Speed.SLOW,
                ),
                after=(ready,),
            )
        if needs_asset_tools:
            ready = plan.add(
                step(
                    "asset-tools",
                    imagebases.MaterializeAssetTools(),
                    contends=(config.exclusive("docker_daemon"),),
                    carry_checks=(imagebases.RequireAssetTools(),),
        kind=Kind.PACKAGE, needs=BUILDS, speed=Speed.SLOW,
                ),
                after=(ready,),
            )
        ready = plan.add(
            assetdependencies.request_step(config, wanted, names, self._args.template),
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

    The graph is invariant: each action checks asset presence only when it
    executes. A warm checkout therefore does no recovery work without hiding
    labels that the private prefix may need to resume.
    """
    arch = config.host_arch()
    recovery = AssetRecovery(config, arch)
    phase = plan.phase("assets")
    names = (arch.name,)
    rust_builders = imagebases.required_rust_builder_names(config, names)
    bases = phase.add(
        _when_missing(
            recovery,
            step(
                "base-images",
                imagebases.Prefetch(names, rust_names=rust_builders, asset_tools=True),
                contends=(config.exclusive("docker_daemon"),),
        kind=Kind.PACKAGE, needs=PULLS, speed=Speed.SLOW,
            ),
        ),
        after=after,
    )
    checked = phase.add(_when_missing(recovery, doctor(config)), after=(bases,))
    ready = phase.add(
        _when_missing(
            recovery,
            step(
                "guest-execution",
                crossexec.Require(names),
                contends=(config.exclusive("docker_daemon"),),
        kind=Kind.PACKAGE, needs=BUILDS, speed=Speed.SLOW,
            ),
        ),
        after=(checked,),
    )
    if rust_builders:
        # Not `_when_missing`. The warm-asset shortcut is about not rebuilding
        # assets; this builds the *tool* that builds them, and
        # `initrd.guest-agents` runs `capsem-builder agent` without asking
        # whether any asset is present. The builder also goes stale on its own
        # schedule -- it is keyed on `Cargo.lock`, so one added dependency
        # invalidates it while every asset on disk stays valid, which is the
        # case the condition cannot see. A run then skipped materialising it
        # and failed four steps later with "locked guest Rust builder is
        # missing".
        #
        # Unconditional costs nothing warm: `materialize_rust_builders` checks
        # `image_exists` and notes that it is already there.
        ready = phase.add(
            step(
                "guest-builders",
                imagebases.MaterializeRustBuilders(rust_builders),
                contends=(config.exclusive("docker_daemon"),),
                carry_checks=(imagebases.RequireRustBuilders(rust_builders),),
                kind=Kind.PACKAGE,
                needs=BUILDS,
                speed=Speed.SLOW,
            ),
            after=(ready,),
        )
    ready = phase.add(
        _when_missing(
            recovery,
            step(
                "asset-tools",
                imagebases.MaterializeAssetTools(),
                contends=(config.exclusive("docker_daemon"),),
                carry_checks=(imagebases.RequireAssetTools(),),
        kind=Kind.PACKAGE, needs=BUILDS, speed=Speed.SLOW,
            ),
        ),
        after=(ready,),
    )
    ready = phase.add(
        _when_missing(
            recovery,
            assetdependencies.dependency_step(
                config, profiles(config), names, label="recovery-dependencies"
            ),
        ),
        after=(ready,),
    )
    images: list[Step] = []
    manifest = config.path(config.imagebuild.output) / config.install.manifest_name
    for profile in profiles(config):
        subject = build(config, profile=profile, arch=arch.name, template="all")
        subject = replace(subject, actions=(Remove(manifest), *subject.actions))
        ready = phase.add(_when_missing(recovery, subject), after=(ready,))
        images.append(ready)
    return tuple(images)


def _when_missing(recovery: AssetRecovery, subject: Step) -> Step:
    return replace(
        subject,
        actions=tuple(recovery.when(action) for action in subject.actions),
        carry_checks=tuple(recovery.when(check) for check in subject.carry_checks),
    )
