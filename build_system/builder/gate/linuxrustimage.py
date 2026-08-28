"""The Linux cross-build image, and the command that warms it.

Split from `hostimage`, which owns the host *builder* image and was over the
three-hundred-line ceiling. Two different images for two different jobs: that
one builds host packages, this one gives a Mac a Linux toolchain to
cross-compile in.
"""

from __future__ import annotations

from . import host, linuxrust
from .actions import Run
from .command import GateCommand
from .config import GateConfig
from .errors import GateError
from .execution import Kind, Needs, Speed, Step, step
from .hostimage import image
from .plan import Plan


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
                    kind=Kind.COMPILE,
                    needs=frozenset({Needs.DOCKER, Needs.DISK}),
                    speed=Speed.SLOW,
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
