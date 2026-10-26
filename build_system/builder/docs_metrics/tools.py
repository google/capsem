"""Resolve the source-pinned scanner through the shared tool cache owner."""

from __future__ import annotations

from pathlib import Path

from ..cache.config import load_paths
from ..cache.tools import materialize
from ..gate.config import for_root


def source_scanner(repo: Path) -> Path:
    return materialize(load_paths(repo), for_root(repo).docs_metrics_scanner).path
