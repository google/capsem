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

from . import host
from .actions import Action, Run
from .command import GateCommand
from .config import GateConfig
from .context import Context
from .errors import GateError
from .execution import Step, step
from .plan import Plan


def _volumes(config: GateConfig) -> list[str]:
    return [
        flag
        for volume in config.hostimage.cached_volumes
        for flag in ("-v", f"{volume.source}:{volume.target}")
    ]


def image(config: GateConfig) -> Step:
    """Build the builder, then prove it can read the checkout as a stranger."""
    settings = config.hostimage
    return step(
        "host-image",
        Run(["docker", "build", "-t", settings.tag, "-f", settings.dockerfile, settings.context]),
        _ForeignUidProbe(),
        contends=(config.exclusive("docker_daemon"),),
    )


class _ForeignUidProbe(Action, name="foreign-uid-probe"):
    """Read the checkout's revision as a user who does not own it."""

    def render(self) -> str:
        return "docker run --user <foreign> ... git rev-parse --short HEAD"

    def perform(self, context: Context) -> None:
        settings = context.config.hostimage
        expected = context.runner.capture(
            ["git", "rev-parse", "--short", "HEAD"], check=False
        )
        if not expected:
            return

        actual = context.runner.capture(
            [
                "docker", "run", "--rm",
                "-v", f"{context.root}:{settings.mount}",
                "-w", settings.mount,
                "--user", settings.probe_user,
                settings.tag,
                "git", "rev-parse", "--short", "HEAD",
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


class LinuxRustCommand(
    GateCommand,
    name="linux-rust",
    help="run the Linux Rust suite natively, or in Docker on macOS",
):
    exclusive = True

    def plan(self) -> Plan:
        plan = Plan(self.name)
        config = self._config
        settings = config.hostimage

        if host.on_linux():
            plan.add(
                step(
                    "linux-rust",
                    Run(
                        ["bash", settings.script],
                        env={"CAPSEM_LINUX_RUST_OUTPUT_DIR": str(config.root)},
                    ),
                )
            )
            return plan

        if not host.on_macos():
            raise GateError(
                "Linux Rust parity runs natively on Linux or in Docker on macOS"
            )

        built = plan.add(image(config))
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
                    ["docker", "run", "--rm", *_volumes(config), settings.tag,
                     "sh", "-c",
                     f"chown -R {uid}:{gid} "
                     + " ".join(v.target for v in settings.cached_volumes)],
                ),
                contends=(docker,),
            ),
            after=(built,),
        )

        suite = plan.add(
            step(
                "linux-rust",
                Run([
                    "docker", "run", "--rm",
                    "--user", f"{uid}:{gid}",
                    *[f for k, v in settings.environment.items() for f in ("-e", f"{k}={v}")],
                    "--tmpfs", settings.tmpfs,
                    "-v", f"{config.root}:{settings.mount}:ro",
                    "-v", f"{output}:{settings.container_output}",
                    "-v", f"{output / settings.nextest_dir}:{settings.mount}/{settings.nextest_mount}",
                    *_volumes(config),
                    "-w", settings.mount,
                    settings.tag,
                    "bash", f"{settings.mount}/{settings.script}",
                ]),
                contends=(docker,),
            ),
            after=(owned,),
        )

        plan.add(
            step(
                "output-ownership",
                Run([
                    "docker", "run", "--rm",
                    "-v", f"{output}:{settings.container_output}",
                    settings.alpine,
                    "chown", "-R", f"{uid}:{gid}", settings.container_output,
                ]),
                contends=(docker,),
            ),
            after=(suite,),
        )
        return plan
