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
from .execution import Kind, Needs, Speed, step
from .fileactions import Copy, MakeDir, Remove
from .lifecycle import Resource
from .plan import Plan


def home(config: GateConfig) -> Path:
    """Where the service keeps its state, honouring an override."""
    override = os.environ.get(config.environment.home)
    return Path(override) if override else host.home() / ".capsem"


def run_dir(config: GateConfig) -> Path:
    override = os.environ.get(config.environment.run_dir)
    return Path(override) if override else home(config) / "run"


class Service(Resource, name="service"):
    """A running daemon, for the length of a phase, inside a given workspace.

    Constructed from the `Workspace` beside it rather than from the ambient
    environment. Resolving `CAPSEM_HOME` at construction meant the daemon could
    be started in one place and stopped in another -- and `acquire` raised
    unconditionally, which made `just smoke` fail on acquisition every time
    rather than confront that.

    Stopped through `pidfiles`, which is the one place in the package allowed
    to send a signal, so "which process did the gate kill" has one answer.
    """

    def __init__(self, config: GateConfig, workspace, runner) -> None:
        self._config = config
        self._workspace = workspace
        # Handed the runner rather than building one. A resource that
        # constructs its own is invisible to whatever is recording the run --
        # which is exactly how a dry run came to execute `git rev-parse`
        # elsewhere in this package.
        self._runner = runner
        self.home = workspace.home
        self.run_dir = workspace.run_dir
        self.started = False

    def environment(self) -> dict[str, str]:
        """Where this service listens, for anything talking to it."""
        return self._config.environment.capsem(home=self.home, run_dir=self.run_dir)

    def acquire(self) -> None:
        """Start the daemon and wait until it is actually listening.

        A pidfile says a process exists; the socket says it is answering, and
        the difference is every flake where a suite raced a daemon that had not
        finished binding.
        """
        from .context import Context

        context = Context(self._runner, self._config, env=self.environment())
        self._stage(context)
        launch(self._config, home=self.home, run_dir=self.run_dir).perform(context)
        WaitForSocket(self.run_dir).perform(context)
        self.started = True

    def _stage(self, context) -> None:
        """Clear a predecessor and put this workspace's config where it looks."""
        settings = self._config.service
        generated = self._config.path(settings.generated_profiles)
        MakeDir(self.run_dir).perform(context)
        pidfiles.stop_gate_service(self.run_dir, self._config.pidfiles)
        Remove(self.run_dir / settings.socket).perform(context)
        Remove(self.home / settings.home_profiles).perform(context)
        Copy(generated, self.home / settings.home_profiles).perform(context)

    def release(self) -> None:
        pidfiles.stop_gate_service(self.run_dir, self._config.pidfiles)


def launch(
    config: GateConfig,
    *,
    home: Path,
    run_dir: Path,
    assets: Path | None = None,
    profiles: Path | None = None,
) -> Launch:
    """Start the daemon, detached, with its pid where `pidfiles` will find it.

    The home and run directory are passed rather than read from the ambient
    environment, so the daemon starts in the workspace that asked for it and
    the thing that stops it is looking at the same place.
    """
    settings = config.service
    names = config.environment
    selected_assets = assets or home / settings.home_assets
    selected_profiles = profiles or config.path(settings.generated_profiles)
    return Launch(
        [
            str(config.path(settings.binary)),
            "--assets-dir",
            str(selected_assets),
            "--process-binary",
            str(config.path(settings.process_binary)),
            "--foreground",
        ],
        pidfile=run_dir / settings.pidfile,
        env={
            names.home: str(home),
            **names.content(assets=selected_assets, profiles=selected_profiles),
            "RUST_LOG": settings.log_level,
        },
    )


class WaitForSocket(Action, name="wait-for-socket"):
    """A pidfile says a process exists; the socket says it is listening."""

    def __init__(self, directory: Path | None = None) -> None:
        self._directory = directory

    def render(self) -> str:
        return "wait for the service socket to accept a connection"

    def perform(self, context: Context) -> None:
        settings = context.config.service
        directory = self._directory or run_dir(context.config)
        path = directory / settings.socket

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
                kind=Kind.CAPSEM,
                needs=frozenset({Needs.DISK}),
                speed=Speed.FAST,
            )
        )

        materialized = plan.add(
            step(
                "materialize",
                Run(
                    [
                        "bash",
                        settings.sync_assets_script,
                        settings.assets_dir,
                        str(target / settings.home_assets),
                    ]
                ),
                Remove(target / settings.home_profiles),
                Copy(generated, target / settings.home_profiles),
                kind=Kind.CAPSEM,
                needs=frozenset({Needs.DISK}),
                speed=Speed.FAST,
            ),
            after=(prepared,),
        )

        plan.add(
            step(
                "start",
                launch(config, home=target, run_dir=run_dir(config)),
                WaitForSocket(),
                kind=Kind.CAPSEM,
                needs=frozenset({Needs.DISK}),
                speed=Speed.FAST,
            ),
            after=(materialized,),
        )
        return plan
