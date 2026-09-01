"""Repository-owned cache policy, inventory, and operations.

The package initializer stays dependency-free because the source-key launcher
imports ``cache.leases`` before the project environment is available. Public
schema conveniences remain lazy for callers that explicitly request them.
"""

from __future__ import annotations

from typing import Any

__all__ = ["CachePaths", "CachePolicy", "PruneMethod", "StagePolicy", "load_policy"]


def __getattr__(name: str) -> Any:
    if name == "load_policy":
        from .config import load_policy

        return load_policy
    if name == "CachePaths":
        from .paths import CachePaths

        return CachePaths
    if name in {"CachePolicy", "PruneMethod", "StagePolicy"}:
        from . import models

        return getattr(models, name)
    raise AttributeError(name)
