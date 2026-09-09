"""Capacity, entry selection, and native mutation locks for one disk owner."""

from __future__ import annotations

from pathlib import Path, PurePosixPath

from pydantic import StrictBool, model_validator

from .contract import CacheContract, CacheScope, PositiveInt, PruneStrategy


def _relative_descendant(value: Path, *, field: str) -> Path:
    posix = PurePosixPath(value.as_posix())
    if value.is_absolute() or str(posix) in {"", "."} or ".." in posix.parts:
        raise ValueError(f"{field} must be a relative descendant")
    return Path(posix)


class StagePolicy(CacheContract):
    """One independently accounted leaf in the cache tree."""

    path: Path
    external: StrictBool = False
    entry_root: Path = Path(".")
    retention_root: Path | None = None
    selector_globs: tuple[str, ...] = ()
    maximum_age_hours: PositiveInt
    maximum_count: PositiveInt | None = None
    managed_globs: tuple[str, ...] = ("*",)
    lease_template: str | None = None
    mutation_locks: tuple[Path, ...] = ()

    @model_validator(mode="after")
    def validate_stage(self) -> StagePolicy:
        if self.path.is_absolute():
            if not self.external:
                raise ValueError("absolute stage path requires external=true")
            if self.prune_strategy is not PruneStrategy.EPHEMERAL:
                raise ValueError("external stage requires the ephemeral prune strategy")
            if len(self.path.parts) < 4:
                raise ValueError("external stage path must name a concrete descendant")
        else:
            if self.external:
                raise ValueError("external stage path must be absolute")
            object.__setattr__(self, "path", _relative_descendant(self.path, field="stage path"))
        entry_root = PurePosixPath(self.entry_root.as_posix())
        if self.entry_root.is_absolute() or ".." in entry_root.parts:
            raise ValueError("stage entry_root must stay inside the stage path")
        object.__setattr__(self, "entry_root", Path(entry_root))
        if self.retention_root is not None:
            object.__setattr__(self, "retention_root", _relative_descendant(
                self.retention_root, field="retention root",
            ))
        object.__setattr__(self, "mutation_locks", tuple(
            _relative_descendant(path, field="mutation lock") for path in self.mutation_locks
        ))
        if self.scope is not CacheScope.DISK:
            raise ValueError("filesystem cache stages require repository scope")
        if self.prune_strategy in {PruneStrategy.DOCKER, PruneStrategy.TART}:
            raise ValueError("filesystem cache stages require a filesystem prune strategy")
        if not self.managed_globs or any(not pattern for pattern in self.managed_globs):
            raise ValueError("managed_globs must contain non-empty patterns")
        if self.lease_template is not None and self.lease_template.count("{key}") != 1:
            raise ValueError("lease_template must contain exactly one {key} placeholder")
        for pattern in self.selector_globs:
            path = PurePosixPath(pattern)
            if not pattern or path.is_absolute() or ".." in path.parts:
                raise ValueError("selector_globs must stay inside the cache root")
        return self
