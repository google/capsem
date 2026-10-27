"""What earlier runs leaked is reaped when the next one starts, not by hand.

Acquired before `CompilerCache`. The cache's shared user locks preserve active
focused compilers, which do not hold the gate's machine lock. Released as a no-op --
what this run leaks is `OrphanAccounting`'s to report, and the next run's to
reap.
"""

from __future__ import annotations

from pathlib import Path

from . import cachelayout
from .config import GateConfig
from .lifecycle import Resource
from .proc import Runner
from .pythonenv import uv_run


def _arguments(config: GateConfig, root: Path) -> tuple[object, ...]:
    return (
        "python", "-m", config.toolchain.reaper_module,
        "--program", config.toolchain.compiler_cache_command,
        "--socket-environment", config.environment.sccache_server_uds,
        "--cache-root", root,
        "--grace-seconds", config.toolchain.reaper_grace_seconds,
    )


def wait_for_shutdown(config: GateConfig, runner: Runner, environment: dict[str, str]) -> None:
    """Observe exit while the caller retains startup and exclusive cache use.

    sccache 0.17 replies ShuttingDown before draining clients and exiting:
    https://github.com/mozilla/sccache/blob/v0.17.0/src/server.rs#L833-L846
    The read-only helper neither acquires the caller's locks nor sends signals.
    """
    runner.run(
        uv_run(
            config, "--no-sync", *_arguments(config, cachelayout.stage_path(config, "rust-sccache")),
            "--wait-only",
        ),
        env=environment,
    )


class StaleProcesses(Resource, name="stale-processes"):
    def __init__(self, config: GateConfig, runner: Runner) -> None:
        self._config = config
        self._runner = runner

    def acquire(self) -> None:
        if self._runner.observing:
            return
        paths = cachelayout.cache_paths(self._config)
        stage = paths.stage("rust-sccache")
        locks = tuple(
            argument
            for lock in paths.policy.stages["rust-sccache"].mutation_locks
            for argument in ("--lease-lock", str(stage / lock))
        )
        self._runner.run(
            uv_run(
                self._config,
                *_arguments(self._config, paths.root),
                *locks,
            )
        )

    def release(self) -> None:
        return None
