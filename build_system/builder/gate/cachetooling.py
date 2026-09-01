"""Language-tool cache selection and warm/cold observation."""

from __future__ import annotations

from pathlib import Path

from ..cache.telemetry import CacheUse, record_use
from ..cache.views import ViewReceipt, canonicalize
from . import cachelayout
from .config import GateConfig


def record_cargo(config: GateConfig, *, key: str, logical_bytes: int) -> CacheUse:
    """Observe the live Cargo profile without deleting compiler internals."""
    return record_use(
        cachelayout.cache_paths(config),
        "cargo-debug",
        tool="cargo",
        key=key,
        logical_bytes=logical_bytes,
    )


def environment(config: GateConfig, *, key: str) -> dict[str, str]:
    """Select keyed tool stages and record their pre-run reuse state."""
    paths = cachelayout.cache_paths(config)
    for stage_id, tool in (
        ("python-uv", "uv"),
        ("python-pycache", "python"),
        ("node-pnpm", "pnpm"),
    ):
        record_use(paths, stage_id, tool=tool, key=key)
    return {
        config.environment.uv_cache: str(
            cachelayout.keyed_stage_path(
                config, "python-uv", *config.toolchain.uv_identity_inputs
            )
        ),
        config.environment.pnpm_store: str(cachelayout.stage_path(config, "node-pnpm")),
    }


def canonicalize_package(config: GateConfig, package: Path) -> ViewReceipt:
    """Bind one named package to its immutable cache object and receipt."""
    return canonicalize(cachelayout.cache_paths(config), package)
