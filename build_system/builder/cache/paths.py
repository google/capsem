"""Resolve policy-owned cache paths without touching the filesystem."""

from __future__ import annotations

from pathlib import Path

from .models import CachePolicy


class CachePaths:
    """Validated absolute views of cache policy paths."""

    def __init__(self, repository_root: Path, policy: CachePolicy) -> None:
        if not repository_root.is_absolute():
            raise ValueError("repository root must be absolute")
        self._repository_root = repository_root.resolve()
        self._policy = policy

    @property
    def root(self) -> Path:
        return self._repository_root / self._policy.root

    def stage(self, stage_id: str) -> Path:
        try:
            stage = self._policy.stages[stage_id]
        except KeyError:
            known = ", ".join(sorted(self._policy.stages))
            raise KeyError(f"unknown cache stage {stage_id!r}; expected one of: {known}") from None
        return self.root / stage.path
