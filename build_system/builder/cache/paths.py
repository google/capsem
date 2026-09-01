"""Resolve policy-owned cache paths without touching the filesystem."""

from __future__ import annotations

from pathlib import Path

from pydantic import BaseModel, ConfigDict, model_validator

from .models import CachePolicy


class CachePaths(BaseModel):
    """Validated absolute views of cache policy paths."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    repository_root: Path
    policy: CachePolicy

    @model_validator(mode="after")
    def validate_root(self) -> CachePaths:
        if not self.repository_root.is_absolute():
            raise ValueError("repository root must be absolute")
        object.__setattr__(self, "repository_root", self.repository_root.absolute())
        return self

    @property
    def root(self) -> Path:
        return self.repository_root / self.policy.root

    def stage(self, stage_id: str) -> Path:
        try:
            stage = self.policy.stages[stage_id]
        except KeyError:
            known = ", ".join(sorted(self.policy.stages))
            raise KeyError(f"unknown cache stage {stage_id!r}; expected one of: {known}") from None
        return self.root / stage.path

    def resolve(self, configured: Path) -> Path:
        """Resolve a repository-relative configured path contained by cache/."""
        if configured.is_absolute():
            raise ValueError("configured path must be repository-relative")
        candidate = (self.repository_root / configured).absolute()
        try:
            candidate.relative_to(self.root)
        except ValueError as error:
            raise ValueError(f"configured path {configured} is outside the cache root") from error
        return candidate
