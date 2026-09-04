"""Language-tool cache selection and warm/cold observation."""

from __future__ import annotations

import os
import shutil
from pathlib import Path

from .. import gatelaunch
from ..cache.leases import retain_generation
from ..cache.pythonenv import PYTEST_ADDOPTS
from ..cache.pythonenv import select as select_python
from ..cache.telemetry import CacheUse, ReuseScope, record_use
from ..cache.views import ViewReceipt, canonicalize
from . import cachelayout
from .config import GateConfig
from .lifecycle import Resource
from .proc import Runner

PYTHONPYCACHEPREFIX = gatelaunch.PYCACHE


def record_cargo(config: GateConfig, *, key: str, logical_bytes: int) -> CacheUse:
    """Observe the live Cargo profile without deleting compiler internals."""
    return record_use(
        cachelayout.cache_paths(config),
        "cargo",
        tool="cargo",
        key=key,
        scope=ReuseScope.SHARED,
        observed_bytes=logical_bytes,
    )


def environment(config: GateConfig, *, key: str, source_root: Path | None = None) -> dict[str, str]:
    """Select keyed tool stages and record their pre-run reuse state."""
    paths = cachelayout.cache_paths(config)
    uv = cachelayout.stage_path(config, "python-uv")
    ruff = cachelayout.stage_path(config, "python-ruff")
    pycache = Path(
        gatelaunch.isolated_environment(
            source_root or config.root, authority=cachelayout.authority(config)
        )[gatelaunch.PYCACHE]
    )
    python = select_python(paths, pycache, inherited_addopts=os.environ.get(PYTEST_ADDOPTS, ""))
    retain_generation(paths, "python-pycache", pycache.name)
    retain_generation(paths, "python-pytest", python.pytest_cache.name)
    retain_generation(paths, "test-temp", python.test_tmp.name)
    record_use(paths, "python-uv", tool="uv", key=key, scope=ReuseScope.SHARED, probe=uv)
    record_use(paths, "python-ruff", tool="ruff", key=key, scope=ReuseScope.SHARED, probe=ruff)
    record_use(
        paths,
        "python-pycache",
        tool="python",
        key=key,
        scope=ReuseScope.GENERATION,
        probe=pycache,
    )
    record_use(
        paths,
        "python-pytest",
        tool="pytest",
        key=key,
        scope=ReuseScope.GENERATION,
        probe=python.pytest_cache,
    )
    record_use(paths, "node-pnpm", tool="pnpm", key=key, scope=ReuseScope.SHARED)
    record_use(
        paths,
        "rust-sccache",
        tool=config.toolchain.compiler_cache_command,
        key=key,
        scope=ReuseScope.SHARED,
        ignored_names=(config.toolchain.compiler_cache_socket_name,),
    )
    return {
        **python.variables(),
        config.environment.uv_cache: str(uv),
        gatelaunch.RUFF_CACHE: str(ruff),
        config.environment.pnpm_store: str(cachelayout.stage_path(config, "node-pnpm")),
    }


def compiler_environment(config: GateConfig) -> dict[str, str]:
    """Enable the pinned compiler cache only after its executable exists."""
    command = config.toolchain.compiler_cache_command
    if shutil.which(command) is None:
        return {}
    paths = cachelayout.cache_paths(config)
    stage = paths.policy.stages["rust-sccache"]
    return {
        config.environment.rustc_wrapper: command,
        config.environment.sccache_dir: str(paths.stage("rust-sccache")),
        config.environment.sccache_cache_size: f"{stage.max_size_bytes // 1024**3}G",
        config.environment.sccache_base_dirs: str(config.root),
        config.environment.sccache_client_side: (
            "1" if config.toolchain.compiler_cache_client_side else "0"
        ),
        config.environment.sccache_idle_timeout: str(
            config.toolchain.compiler_cache_idle_timeout_seconds
        ),
        config.environment.sccache_server_uds: str(
            paths.stage("rust-sccache") / config.toolchain.compiler_cache_socket_name
        ),
    }


class CompilerCache(Resource, name="compiler-cache"):
    """Keep one correctly scoped sccache server alive for a gate command."""

    def __init__(self, config: GateConfig, runner: Runner) -> None:
        self._config = config
        self._runner = runner
        self._environment = compiler_environment(config)
        self._active = False

    def acquire(self) -> None:
        from .context import Context
        from .fileactions import MakeDir

        if not self._environment or self._runner.observing:
            return
        stage = cachelayout.stage_path(self._config, "rust-sccache")
        MakeDir(stage).perform(Context(self._runner, self._config, env=self._environment))
        command = self._config.toolchain.compiler_cache_command
        self._runner.succeeds((command, "--stop-server"), env=self._environment)
        self._runner.run((command, "--start-server"), env=self._environment)
        self._active = True

    def release(self) -> None:
        if not self._active:
            return
        self._runner.run(
            (self._config.toolchain.compiler_cache_command, "--stop-server"),
            env=self._environment,
        )
        self._active = False

    def environment(self) -> dict[str, str]:
        return self._environment


def canonicalize_package(config: GateConfig, package: Path) -> ViewReceipt:
    """Bind one named package to its immutable cache object and receipt."""
    return canonicalize(cachelayout.cache_paths(config), package)
