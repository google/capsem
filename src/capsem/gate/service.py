"""The development daemon: started, waited for, and stopped by its pidfile.

`_ensure-service` was fifty-eight lines, and the careful parts were all
comments. It stopped only the service its own pidfile named -- never `pkill` --
because killing by pattern takes down a developer's installed capsem, or a
parallel run with a different `CAPSEM_HOME`. It launched the daemon with `3>&-`
so the backgrounded process would not inherit the execution-lock descriptor and
hold the flock after the outer shell exited.

Both are structural here. Stopping goes through `pidfiles`, the one module
allowed to send a signal, and the launch closes its inherited descriptors
because Python does that by default -- with a test in `test_gate_locks.py`
holding it.
"""

from __future__ import annotations

import os
import socket
import time
from pathlib import Path

from . import host, pidfiles
from .actions import Action, Launch, Run
from .command import GateCommand
from .config import GateConfig
from .context import Context
from .errors import GateError
from .execution import step
from .fileactions import Copy, MakeDir, Remove
from .lifecycle import Resource
from .plan import Plan


def home(config: GateConfig) -> Path:
    """Where the service keeps its state, honouring an override."""
    override = os.environ.get("CAPSEM_HOME")
    return Path(override) if override else host.home() / ".capsem"


def run_dir(config: GateConfig) -> Path:
    override = os.environ.get("CAPSEM_RUN_DIR")
    return Path(override) if override else home(config) / "run"


class Service(Resource, name="service"):
    """A running daemon, for the length of a phase.

    Stopped through `pidfiles`, which is the one place in the package allowed
    to send a signal -- so "which process did the gate kill" has one answer.
    """

    def __init__(self, config: GateConfig) -> None:
        self._config = config
        self.run_dir = run_dir(config)

    def acquire(self) -> None:
        raise GateError("start the service through its plan, not by acquiring it")

    def release(self) -> None:
        pidfiles.stop_gate_service(self.run_dir, self._config.pidfiles)


def _launch(config: GateConfig) -> Launch:
    """Start the daemon, detached, with its pid where `pidfiles` will find it."""
    settings = config.service
    target = home(config)
    return Launch(
        [
            str(config.path(settings.binary)),
            "--assets-dir", str(target / settings.home_assets),
            "--process-binary", str(config.path(settings.process_binary)),
            "--foreground",
        ],
        pidfile=run_dir(config) / settings.pidfile,
        env={
            "CAPSEM_HOME": str(target),
            "CAPSEM_PROFILES_DIR": str(config.path(settings.generated_profiles)),
            "RUST_LOG": settings.log_level,
        },
    )


class _WaitForSocket(Action, name="wait-for-socket"):
    """A pidfile says a process exists; the socket says it is listening."""

    def render(self) -> str:
        return "wait for the service socket to accept a connection"

    def perform(self, context: Context) -> None:
        settings = context.config.service
        path = run_dir(context.config) / settings.socket

        for _ in range(settings.ready_attempts):
            if path.exists():
                probe = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                try:
                    probe.connect(str(path))
                    return
                except OSError:
                    pass
                finally:
                    probe.close()
            time.sleep(settings.ready_interval_seconds)

        raise GateError(
            f"capsem-service did not accept a connection on {path} within "
            f"{settings.ready_attempts * settings.ready_interval_seconds:.0f}s"
        )


class _StopExisting(Action, name="stop-existing-service"):
    """Stop only what this run's pidfile names.

    Never by pattern: that takes down a developer's installed capsem, or a
    parallel run with a different `CAPSEM_HOME`.
    """

    def render(self) -> str:
        return "stop the service this run's pidfile names, if any"

    def perform(self, context: Context) -> None:
        directory = run_dir(context.config)
        pidfiles.stop_gate_service(directory, context.config.pidfiles)
        Remove(directory / context.config.service.socket).perform(context)


class EnsureServiceCommand(
    GateCommand, name="ensure-service", help="start the development daemon idempotently"
):
    exclusive = True

    def plan(self) -> Plan:
        plan = Plan(self.name)
        config = self._config
        settings = config.service
        target = home(config)
        generated = config.path(settings.generated_profiles)

        if not generated.is_dir():
            raise GateError(
                f"generated profiles are missing at {generated}; run "
                "`just _materialize-config` or a recipe that depends on it"
            )

        prepared = plan.add(
            step(
                "prepare",
                MakeDir(run_dir(config)),
                _StopExisting(),
                # An older layout wrote these into the home. Removed on every
                # start, so a checkout that predates the change cannot keep
                # booting from them.
                *[Remove(target / name) for name in settings.retired_config],
            )
        )

        materialized = plan.add(
            step(
                "materialize",
                Run([
                    "bash", settings.sync_assets_script,
                    settings.assets_dir,
                    str(target / settings.home_assets),
                ]),
                Remove(target / settings.home_profiles),
                Copy(generated, target / settings.home_profiles),
            ),
            after=(prepared,),
        )

        plan.add(
            step("start", _launch(config), _WaitForSocket()),
            after=(materialized,),
        )
        return plan
