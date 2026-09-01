"""Language-tool cache selection and warm/cold observation."""

from __future__ import annotations

import shutil
from pathlib import Path

from ..cache.telemetry import CacheUse, record_use
from ..cache.views import ViewReceipt, canonicalize
from . import cachelayout
from .config import GateConfig


def record_cargo(config: GateConfig, *, key: str, logical_bytes: int) -> CacheUse:
    """Observe the live Cargo profile without deleting compiler internals."""
    return record_use(
        cachelayout.cache_paths(config),
        "cargo",
        tool="cargo",
        key=key,
        logical_bytes=logical_bytes,
    )


def environment(config: GateConfig, *, key: str) -> dict[str, str]:
    """Select keyed tool stages and record their pre-run reuse state."""
    paths = cachelayout.cache_paths(config)
    uv = cachelayout.keyed_stage_path(config, "python-uv", *config.toolchain.uv_identity_inputs)
    record_use(paths, "python-uv", tool="uv", key=key, probe=uv)
    for stage_id, tool in (
        ("python-pycache", "python"),
        ("node-pnpm", "pnpm"),
        ("rust-sccache", config.toolchain.compiler_cache_command),
    ):
        record_use(paths, stage_id, tool=tool, key=key)
    return {
        config.environment.uv_cache: str(uv),
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
        config.environment.sccache_cache_size: f"{stage.hard_bytes // 1024**3}G",
        config.environment.sccache_base_dir: str(config.root),
    }


def canonicalize_package(config: GateConfig, package: Path) -> ViewReceipt:
    """Bind one named package to its immutable cache object and receipt."""
    return canonicalize(cachelayout.cache_paths(config), package)
