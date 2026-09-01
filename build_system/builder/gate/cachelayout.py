"""Resolve stable shared gate roots through the typed cache library."""

from __future__ import annotations

import os
from pathlib import Path

from ..cache.config import load_policy
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
    paths = CachePaths(repository_root=authority(config), policy=load_policy(config.root))
    try:
        return paths.resolve(path)
    except ValueError as error:
        raise GateError(str(error)) from error
