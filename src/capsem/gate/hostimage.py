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

from . import host, linuxrust
from .actions import Action, Run
from .command import GateCommand
from .config import GateConfig
from .context import Context
from .docker import Docker
from .dockermount import Mount
from .errors import GateError
from .execution import Step, step
from .gitmetadata import docker_git_metadata_mount
from .plan import Plan

#: One name, so every lane that needs the builder depends on the same step
#: rather than each spelling its own label.
STEP = "host-image"


def image(config: GateConfig) -> Step:
    """Build the builder, then prove it can read the checkout as a stranger."""
    return step(
        STEP,
        _Build(),
        _ForeignUidProbe(),
        contends=(config.exclusive("docker_daemon"),),
    )


class _Build(Action, name="host-image-build"):
    """Build the builder, through the wrapper like every other image."""

    def render(self) -> str:
        return "docker build the Linux host builder image"

    def perform(self, context: Context) -> None:
        settings = context.config.hostimage
        Docker(context.runner).build(
            tag=settings.tag, dockerfile=settings.dockerfile, context=settings.context
        )


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


class _ForeignUidProbe(Action, name="foreign-uid-probe"):
    """Read the checkout's revision as a user who does not own it."""

    def render(self) -> str:
        return "docker run --user <foreign> ... git rev-parse --short HEAD"

    def perform(self, context: Context) -> None:
        settings = context.config.hostimage
        expected = context.runner.capture(["git", "rev-parse", "--short", "HEAD"], check=False)
        if not expected:
            return

        metadata = docker_git_metadata_mount(context.runner)
        actual = Docker(context.runner).read(
            image=settings.tag,
            command=["git", "rev-parse", "--short", "HEAD"],
            # It reads a revision out of a checkout. Declared rather than
            # omitted, which is how every container in the gate used to have
            # outbound access without anyone choosing it.
            network=settings.probe_network,
            options=("--user", settings.probe_user),
            mounts=(
                Mount.unmigrated(str(context.root), settings.mount, "ro"),
                *((metadata,) if metadata is not None else ()),
            ),
            workdir=settings.mount,
            check=False,
        )
        if actual != expected:
            raise GateError(
                f"{settings.tag} cannot read {settings.mount} as a non-owner user, "
                "so Linux package builds would embed an 'unknown' build hash. "
                f"Keep `git config --system --add safe.directory {settings.mount}` "
                f"in {settings.dockerfile}."
            )
        context.journal.note(f"host-builder reads {settings.mount} as a stranger ({actual})")


def linux_rust(plan: Plan, config: GateConfig, *, after: tuple[Step, ...] = ()) -> Step:
    """The Linux Rust parity lane, composed into an existing plan.

    Native Linux exercises the `cfg(target_os = "linux")` branches directly; a
    Mac host runs the same checked-in script in Docker, or Linux-only
    regressions stay out of the local gate entirely.
    """
    return _LinuxRust(plan, config).build(after)


class _LinuxRust:
    """Builds the parity lane's steps into whichever plan is composing it."""

    def __init__(self, plan: Plan, config: GateConfig) -> None:
        self._plan = plan
        self._config = config

    def build(self, after: tuple[Step, ...]) -> Step:
        config = self._config
        plan = self._plan

        if host.on_linux():
            return plan.add(
                step(
                    "linux-rust",
                    Run(
                        ["bash", config.hostimage.script],
                        env={config.environment.linux_rust.output_dir: str(config.root)},
                    ),
                ),
                after=after,
            )

        # Named, not inferred from "not Linux". Without this a third platform
        # falls through to the Docker path and fails somewhere inside a
        # container instead of saying which host it will not run on.
        if not host.on_macos():
            raise GateError("Linux Rust parity runs natively on Linux or in Docker on macOS")

        # macOS: the same checked-in script, in a container that holds its own
        # copy of the source. `cache-ownership`, `linux-rust-mountpoints` and
        # `output-ownership` are gone with the mounts and volumes that
        # required them -- they existed only to repair what root-owned shared
        # state left behind.
        built = plan.shared(image(config), after=after)
        return linuxrust.lane(plan, config, after=(built,))


class LinuxRustCommand(
    GateCommand,
    name="linux-rust",
    help="run the Linux Rust suite natively, or in Docker on macOS",
):
    exclusive = True

    def plan(self) -> Plan:
        plan = Plan(self.name)
        linux_rust(plan, self._config)
        return plan
