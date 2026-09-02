"""Resolve stable shared gate roots through the typed cache library."""

from __future__ import annotations

import os
from pathlib import Path

from ..cache.config import load_policy
from ..cache.models import StagePolicy
from ..cache.paths import CachePaths
from .config import GateConfig
from .errors import GateError


def authority(config: GateConfig) -> Path:
    """Return the outer checkout that owns shared cache state."""
    raw = os.environ.get(config.environment.source_checkout)
    root = Path(raw) if raw else config.root
    if not root.is_absolute():
        raise GateError(f"cache authority must be absolute: {root}")
    return root.absolute()


def shared_path(config: GateConfig, configured: str | Path) -> Path:
    """Resolve one configured shared path, with absolute test overrides explicit."""
    path = Path(configured).expanduser()
    if path.is_absolute():
        return path
    paths = cache_paths(config)
    try:
        return paths.resolve(path)
    except ValueError as error:
        raise GateError(str(error)) from error


def stage_path(config: GateConfig, stage_id: str) -> Path:
    """Resolve a policy stage against the outer cache authority."""
    paths = cache_paths(config)
    try:
        return paths.stage(stage_id)
    except KeyError as error:
        raise GateError(str(error)) from error


def stage_policy(config: GateConfig, stage_id: str) -> StagePolicy:
    """Return one disk cache contract from the sole cache authority."""
    try:
        return load_policy(config.root).stages[stage_id]
    except KeyError as error:
        raise GateError(f"cache policy has no disk owner {stage_id!r}") from error


def cache_paths(config: GateConfig) -> CachePaths:
    """Return the typed path authority shared by gate cache adapters."""
    return CachePaths(repository_root=authority(config), policy=load_policy(config.root))
