"""The Linux builder image, and the parity lane that runs inside it.

Native Linux exercises the `cfg(target_os = "linux")` branches directly. A Mac
host has to run the same checked-in script in Docker, or Linux-only
regressions stay out of the local gate entirely and surface first in the
release job that owns them.

The foreign-UID probe is the interesting part. On Linux CI the checkout's owner
is not the image's user, so git rejects `/src` as dubious ownership -- and
`build.rs` answers that by embedding `unknown` rather than failing, which is
how a binary with no source identity reaches the provenance check. Forcing a
foreign UID reproduces it here, and works on macOS too because git compares
`st_uid` to `euid` in userspace rather than trusting the mount.
"""

from __future__ import annotations

import hashlib
from pathlib import Path

from .actions import Action
from .command import GateCommand
from .config import GateConfig
from .context import Context
from .docker import Docker
from .errors import GateError
from .execution import Kind, Needs, Speed, Step, step
from .invocation import ConsoleMode
from .outside import Outside
from .packageinputs import pinned_toolchain
from .plan import Plan

#: One name, so every lane that needs the builder depends on the same step
#: rather than each spelling its own label.
STEP = "host-image"
INPUT_KEY_LABEL = "org.capsem.host-builder.input-key"


def cargo_tool(*, config: GateConfig, argument: str) -> tuple[str, str]:
    """The install package and version named by one Docker build argument."""
    try:
        name = config.hostimage.cargo_tool_args[argument]
    except KeyError:
        raise GateError(f"unknown host-builder Cargo tool argument {argument!r}") from None
    matches = [crate for crate in config.toolchain.crates if crate.name == name]
    if len(matches) != 1:
        raise GateError(f"host-builder Cargo tool {name!r} is not uniquely configured")
    install = matches[0].install
    version_at = install.index("--version") + 1
    return install[2], install[version_at]


def _identity_files(config: GateConfig) -> tuple[Path, ...]:
    files = tuple(config.path(relative) for relative in config.hostimage.builder_identity_inputs)
    missing = [path for path in files if not path.is_file()]
    if missing:
        raise GateError(
            "host-builder identity inputs are missing: " + ", ".join(str(path) for path in missing)
        )
    return files


def input_key(config: GateConfig) -> str:
    """Digest every file and config value that can change the builder."""
    settings = config.hostimage
    digest = hashlib.blake2b(digest_size=16)
    for path in _identity_files(config):
        digest.update(path.relative_to(config.root).as_posix().encode())
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    values = [
        config.host_arch().docker_platform,
        config.apt_snapshot.base,
        config.apt_snapshot.id,
        settings.materialize_network,
        settings.pnpm_version,
        settings.rust_image,
        settings.uv_image,
        pinned_toolchain(config.root),
        *config.toolchain.rust_targets,
        *config.toolchain.linux.apt_packages,
        *_cross_apt_packages(config),
        *config.toolchain.linux.pkg_config_modules,
        *config.toolchain.linux.required_commands,
    ]
    for argument in sorted(settings.cargo_tool_args):
        package, version = cargo_tool(config=config, argument=argument)
        values.extend((argument, settings.cargo_tool_args[argument], package, version))
    for value in values:
        digest.update(value.encode())
        digest.update(b"\0")
    return digest.hexdigest()


def _cross_apt_packages(config: GateConfig) -> tuple[str, ...]:
    """Every config-owned GNU compiler needed by either concrete target."""
    return tuple(
        package
        for architecture in config.architectures.values()
        for package in architecture.apt_cross_compilers
    )


def _image_tag(ref: str) -> str:
    """Version tag from a schema-validated digest-qualified image ref."""
    return ref.split("@", 1)[0].rsplit(":", 1)[1]


def _prove_tools(context: Context, docker: Docker) -> None:
    """Execute every promised tool inside the exact image, without egress."""
    config = context.config
    settings = config.hostimage
    probes: list[tuple[tuple[str, ...], str]] = [
        (("rustc", "--version"), f"rustc {pinned_toolchain(config.root)} "),
        (("uv", "--version"), f"uv {_image_tag(settings.uv_image)}"),
        (("pnpm", "--version"), settings.pnpm_version),
    ]
    for argument in sorted(settings.cargo_tool_args):
        name = settings.cargo_tool_args[argument]
        tool = next(crate for crate in config.toolchain.crates if crate.name == name)
        probes.append((tool.probe, tool.expected))
    script = (
        'expected=$1; shift; actual=$("$@" 2>&1); case "$actual" in "$expected"*) exit 0 ;; '
        '*) printf "expected %s, got %s\\n" "$expected" "$actual" >&2; exit 1 ;; esac'
    )
    context.runner.step("Proving exact host-builder tools without network")
    for probe, expected in probes:
        if not docker.probe(
            image=settings.tag,
            command=["sh", "-eu", "-c", script, "host-builder-probe", expected, *probe],
            network=settings.network,
        ):
            raise GateError(
                f"host builder {settings.tag} does not provide {expected!r} through {probe!r}"
            )


def image(config: GateConfig) -> Step:
    """Build the builder, then prove it can read the checkout as a stranger."""
    return step(
        STEP,
        Outside(_Build()),
        _Require(),
        contends=(config.exclusive("docker_daemon"),),
        carry_checks=(_Require(),),
        kind=Kind.COMPILE,
        needs=frozenset({Needs.DOCKER, Needs.DISK}),
        speed=Speed.SLOW,
    )


class _Build(Action, name="host-image-materialize"):
    """Materialize the builder's exact dependencies at its named egress edge."""

    def render(self) -> str:
        return "docker build the Linux host builder image"

    def perform(self, context: Context) -> None:
        settings = context.config.hostimage
        docker = Docker(context.runner)
        platform = context.config.host_arch().docker_platform
        identity = input_key(context.config)
        found: str | None = None
        if docker.image_exists(settings.tag, platform=platform):
            found = docker.image_label(settings.tag, INPUT_KEY_LABEL)
            if found == identity:
                context.runner.note(f"host builder input key is already present: {identity}")
            else:
                context.runner.note(
                    f"host builder input changed from {found or '<unlabelled>'} to {identity}"
                )
        if found != identity:
            arguments = [
                f"APT_SNAPSHOT_BASE={context.config.apt_snapshot.base}",
                f"APT_SNAPSHOT_ID={context.config.apt_snapshot.id}",
                f"PNPM_VERSION={settings.pnpm_version}",
                f"RUST_IMAGE={settings.rust_image}",
                f"UV_IMAGE={settings.uv_image}",
                f"RUST_TOOLCHAIN={pinned_toolchain(context.config.root)}",
                "RUST_TARGETS=" + " ".join(context.config.toolchain.rust_targets),
                f"INPUT_IDENTITY={identity}",
                "WORKSPACE_APT_PACKAGES=" + " ".join(context.config.toolchain.linux.apt_packages),
                "WORKSPACE_CROSS_APT_PACKAGES=" + " ".join(_cross_apt_packages(context.config)),
            ]
            for argument in sorted(settings.cargo_tool_args):
                _package, version = cargo_tool(config=context.config, argument=argument)
                arguments.append(f"{argument}={version}")
            context.runner.step("Materializing exact host-builder dependencies")
            docker.build(
                tag=settings.tag,
                dockerfile=settings.dockerfile,
                context=settings.context,
                args=arguments,
                platform=platform,
                network=settings.materialize_network,
                console=ConsoleMode.LOG_ONLY,
            )
            found = docker.image_label(settings.tag, INPUT_KEY_LABEL)
            if found != identity:
                raise GateError(
                    f"host builder {settings.tag} carries input key {found!r}, expected {identity}"
                )
            context.runner.note(f"host builder materialized with input key {identity}")


class _Require(Action, name="host-image-require"):
    """Prove the carried builder still names and executes the exact inputs."""

    def render(self) -> str:
        return "require the carried exact Linux host builder image"

    def perform(self, context: Context) -> None:
        settings = context.config.hostimage
        docker = Docker(context.runner)
        platform = context.config.host_arch().docker_platform
        identity = input_key(context.config)
        if not docker.image_exists(settings.tag, platform=platform):
            raise GateError(f"host builder {settings.tag} is missing")
        found = docker.image_label(settings.tag, INPUT_KEY_LABEL)
        if found != identity:
            raise GateError(
                f"host builder {settings.tag} carries input key {found!r}, expected {identity}"
            )
        _prove_tools(context, docker)


def fragment(plan: Plan, config: GateConfig, *, after: tuple[Step, ...] = ()) -> Step:
    """Make the builder image available in this plan, building it once.

    Composed rather than dispatched. Both `install-image` and `cross-compile`
    used to run `just _build-host-image` -- a recipe that has never existed, so
    both were broken at runtime and neither test noticed, because both stopped
    at the recipe boundary instead of crossing it.

    `shared`, so two lanes in one plan get a diamond rather than a duplicate
    label or a six-gigabyte image built twice.
    """
    return plan.shared(image(config), after=after)


class HostImageCommand(
    GateCommand,
    name="host-image",
    help="materialize the exact Linux host-builder dependency image",
):
    """Focused cold/warm acceptance without continuing into package proof."""

    # Its plan builds an image outside the kernel sandbox, which needs the
    # egress resource to run it with.
    outside_egress = True

    exclusive = True

    def plan(self) -> Plan:
        plan = Plan(self.name)
        fragment(plan, self._config)
        return plan
