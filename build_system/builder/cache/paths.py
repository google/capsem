"""Resolve policy-owned cache paths without touching the filesystem."""

from __future__ import annotations

import hashlib
from pathlib import Path

from pydantic import BaseModel, ConfigDict, model_validator

from .models import CachePolicy


def _external_namespace(repository_root: Path) -> str:
    return hashlib.sha256(str(repository_root.absolute()).encode()).hexdigest()[:8]


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
        repository = self.repository_root.absolute()
        resolved = sorted(
            (stage_id, self.stage(stage_id).absolute()) for stage_id in self.policy.stages
        )
        for stage_id, path in resolved:
            stage = self.policy.stages[stage_id]
            if stage.external and (
                path == repository or path in repository.parents or repository in path.parents
            ):
                raise ValueError(
                    f"external cache stage {stage_id!r} must be outside the repository"
                )
        for index, (left_id, left) in enumerate(resolved):
            for right_id, right in resolved[index + 1 :]:
                if left == right or left in right.parents or right in left.parents:
                    raise ValueError(
                        f"resolved cache stage paths overlap: {left_id}={left} and {right_id}={right}"
                    )
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
        if stage.external:
            return stage.path / _external_namespace(self.repository_root)
        return self.root / stage.path

    def contained_entry(self, stage_id: str, target: Path) -> Path:
        """Resolve one removable child beneath its exact configured stage."""
        stage_root = self.stage(stage_id).absolute()
        absolute_target = target.absolute()
        if absolute_target == stage_root or stage_root not in absolute_target.parents:
            raise ValueError(f"refusing target outside cache stage {stage_id!r}: {target}")
        return absolute_target

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
