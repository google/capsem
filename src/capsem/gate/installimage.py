"""The disposable image the install proof runs inside.

Always rebuilt from its checked-in Dockerfile, never merely checked for. Docker
keeps unchanged layers cached, so the rebuild is nearly free -- while asking
whether the tag exists lets a stale local image hide a new CI prerequisite, and
then the gate proves an environment nobody else has.

The smoke check exists because a cached layer can satisfy `docker build` and
still be missing a tool. One cacheless rebuild is attempted when it fails; a
second failure is a defect in the Dockerfile rather than a stale layer, and
saying so is more use than a third attempt.
"""

from __future__ import annotations

import argparse

from . import config as gate_config
from .errors import GateError
from .proc import Runner
from .storage import Storage

# The one behaviour here rather than in config: a check, not a value. It
# exercises every tool the install gate depends on -- passwordless sudo,
# cdxgen, the musl toolchain, and a pytest that can actually collect.
SMOKE = (
    "set -e; sudo -n true; cd /src; cdxgen --version; "
    "source /src/scripts/doctor-linux.sh; linux_musl_toolchain_available; "
    "uv run python -m pytest --version; "
    "uv run python -m pytest -p no:cacheprovider -q tests/test_materialize_config_http.py"
)


def _smoke_passes(runner: Runner, settings: gate_config.InstallConfig) -> bool:
    return runner.succeeds(
        [
            "docker", "run", "--rm",
            "-u", "capsem",
            "-e", f"UV_PROJECT_ENVIRONMENT={settings.venv}",
            "-e", f"CAPSEM_TEST_OUTPUT_ROOT={settings.test_output_root}",
            "-v", f"{runner.root}:{settings.mount}:ro",
            settings.image,
            "bash", "-lc", SMOKE,
        ]
    )


def prepare(runner: Runner) -> None:
    """Build the install-test image and prove it can run the gate's tools."""
    settings = gate_config.for_root(runner.root).install
    build = ["docker", "build", "-t", settings.image, "-f", settings.dockerfile, "."]

    runner.run(["just", "_build-host-image"])
    runner.run(build)

    if not _smoke_passes(runner, settings):
        runner.note(
            "Install-test image smoke check failed; rebuilding without Docker cache..."
        )
        runner.run([*build[:2], "--no-cache", *build[2:]])
        if not _smoke_passes(runner, settings):
            raise GateError(
                f"{settings.dockerfile} produces an image that cannot run the install "
                "gate's tools even after a cacheless rebuild"
            )

    # The cross-compile lanes that follow stage 0 reuse the base image. Every
    # other caller only executes the verified derived image, so release the
    # separate ~6 GiB base tag before their package and runtime work.
    Storage(runner).release("linux-rust-builder")


def register(subparsers: argparse._SubParsersAction) -> None:
    parser = subparsers.add_parser(
        "install-image", help="build and smoke the disposable install-test image"
    )
    parser.set_defaults(handler=_command)


def _command(args: argparse.Namespace, runner: Runner) -> int:
    prepare(runner)
    return 0
