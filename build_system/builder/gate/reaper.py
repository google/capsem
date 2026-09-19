"""What earlier runs leaked is reaped when the next one starts, not by hand.

Acquired under the machine lock and before `CompilerCache`: under the lock no
other run is compiling, so every server in this repository's cache is stale,
and the one this run needs has not been started yet. Released as a no-op --
what this run leaks is `OrphanAccounting`'s to report, and the next run's to
reap.
"""

from __future__ import annotations

from . import cachelayout
from .config import GateConfig
from .lifecycle import Resource
from .proc import Runner
from .pythonenv import uv_run


class StaleProcesses(Resource, name="stale-processes"):
    def __init__(self, config: GateConfig, runner: Runner) -> None:
        self._config = config
        self._runner = runner

    def acquire(self) -> None:
        if self._runner.observing:
            return
        toolchain = self._config.toolchain
        self._runner.run(
            uv_run(
                self._config,
                "python",
                "-m",
                toolchain.reaper_module,
                "--program",
                toolchain.compiler_cache_command,
                "--socket-environment",
                self._config.environment.sccache_server_uds,
                "--cache-root",
                cachelayout.cache_paths(self._config).root,
                "--grace-seconds",
                toolchain.reaper_grace_seconds,
            )
        )

    def release(self) -> None:
        return None
