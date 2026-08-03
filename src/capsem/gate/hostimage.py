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

import os
from pathlib import Path

from . import host
from .actions import Action, Run
from .command import GateCommand
from .config import GateConfig
from .context import Context
from .errors import GateError
from .execution import Step, step
from .fileactions import MakeDir
from .gitmetadata import docker_git_metadata_mount
from .plan import Plan


def _volumes(config: GateConfig) -> list[str]:
    return [
        flag
        for volume in config.hostimage.cached_volumes
        for flag in ("-v", f"{volume.source}:{volume.target}")
    ]


#: One name, so every lane that needs the builder depends on the same step
#: rather than each spelling its own label.
STEP = "host-image"


def image(config: GateConfig) -> Step:
    """Build the builder, then prove it can read the checkout as a stranger."""
    settings = config.hostimage
    return step(
        STEP,
        Run(["docker", "build", "-t", settings.tag, "-f", settings.dockerfile, settings.context]),
        _ForeignUidProbe(),
        contends=(config.exclusive("docker_daemon"),),
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

        actual = context.runner.capture(
            [
                "docker",
                "run",
                "--rm",
                "-v",
                f"{context.root}:{settings.mount}",
                *docker_git_metadata_mount(context.runner),
                "-w",
                settings.mount,
                "--user",
                settings.probe_user,
                settings.tag,
                "git",
                "rev-parse",
                "--short",
                "HEAD",
            ],
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
        settings = config.hostimage
        plan = self._plan

        if host.on_linux():
            return plan.add(
                step(
                    "linux-rust",
                    Run(
                        ["bash", settings.script],
                        env={config.environment.linux_rust.output_dir: str(config.root)},
                    ),
                ),
                after=after,
            )

        if not host.on_macos():
            raise GateError("Linux Rust parity runs natively on Linux or in Docker on macOS")

        built = fragment(plan, config, after=after)
        output = config.path(settings.output_dir)
        uid, gid = os.getuid(), os.getgid()
        docker = config.exclusive("docker_daemon")

        # The cached volumes belong to root until they are handed over; the
        # suite then runs as the host user, because running it as container
        # root makes chmod-based permission regressions impossible to observe.
        owned = plan.add(
            step(
                "cache-ownership",
                Run(
                    [
                        "docker",
                        "run",
                        "--rm",
                        *_volumes(config),
                        settings.tag,
                        "sh",
                        "-c",
                        f"chown -R {uid}:{gid} "
                        + " ".join(v.target for v in settings.cached_volumes),
                    ],
                ),
                contends=(docker,),
            ),
            after=(built,),
        )

        mountpoints = plan.add(
            step(
                "linux-rust-mountpoints",
                MakeDir(config.path(settings.nextest_mount)),
                MakeDir(output / settings.nextest_dir),
                *(
                    action
                    for volume in settings.writable_source_mounts
                    for action in (
                        MakeDir(config.path(volume.source)),
                        MakeDir(config.path(volume.target)),
                    )
                ),
            ),
            after=(owned,),
        )

        suite = plan.add(
            step(
                "linux-rust",
                _LinuxRustSuite(
                    output,
                    source=config.root,
                    mount=settings.mount,
                    script=settings.script,
                ),
                contends=(docker,),
            ),
            after=(mountpoints,),
        )

        return plan.add(
            step(
                "output-ownership",
                Run(
                    [
                        "docker",
                        "run",
                        "--rm",
                        "-v",
                        f"{output}:{settings.container_output}",
                        settings.alpine,
                        "chown",
                        "-R",
                        f"{uid}:{gid}",
                        settings.container_output,
                    ]
                ),
                contends=(docker,),
            ),
            after=(suite,),
        )


class _LinuxRustSuite(Action, name="linux-rust-suite"):
    """Run the Linux parity script with runtime-resolved worktree metadata."""

    def __init__(self, output: Path, *, source: Path, mount: str, script: str) -> None:
        self._output = output
        self._source = source
        self._mount = mount
        self._script = script

    def render(self) -> str:
        return (
            f"docker run --user <host> -v {self._source}:{self._mount}:ro "
            f"... bash {self._mount}/{self._script}"
        )

    def perform(self, context: Context) -> None:
        settings = context.config.hostimage
        output = self._output
        uid, gid = host.user()
        context.runner.run(
            [
                "docker",
                "run",
                "--rm",
                "--user",
                f"{uid}:{gid}",
                *[f for k, v in settings.environment.items() for f in ("-e", f"{k}={v}")],
                "--tmpfs",
                settings.tmpfs,
                "-v",
                f"{context.root}:{settings.mount}:ro",
                *docker_git_metadata_mount(context.runner),
                "-v",
                f"{output}:{settings.container_output}",
                "-v",
                f"{output / settings.nextest_dir}:{settings.mount}/{settings.nextest_mount}",
                *[
                    flag
                    for volume in settings.writable_source_mounts
                    for flag in (
                        "-v",
                        f"{context.config.path(volume.source)}:"
                        f"{settings.mount}/{volume.target}",
                    )
                ],
                *_volumes(context.config),
                "-w",
                settings.mount,
                settings.tag,
                "bash",
                f"{settings.mount}/{settings.script}",
            ]
        )

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
