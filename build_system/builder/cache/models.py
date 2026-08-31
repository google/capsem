"""Strict immutable models for the cache control plane."""

from __future__ import annotations

import re
from enum import StrEnum
from pathlib import Path, PurePosixPath
from typing import Annotated

from pydantic import BaseModel, ConfigDict, Field, StrictBool, StrictInt, model_validator

PositiveStrictInt = Annotated[StrictInt, Field(gt=0)]
STAGE_ID = re.compile(r"^[a-z][a-z0-9-]*$")


class PruneMethod(StrEnum):
    """Supported retention algorithms."""

    LRU = "lru"
    GENERATIONAL = "generational"
    EPHEMERAL = "ephemeral"
    EXTERNAL = "external"
    NONE = "none"


def _relative_descendant(value: Path, *, field: str) -> Path:
    posix = PurePosixPath(value.as_posix())
    if value.is_absolute() or str(posix) in {"", "."} or ".." in posix.parts:
        raise ValueError(f"{field} must be a relative descendant")
    return Path(posix)


class CalibrationPolicy(BaseModel):
    """Evidence requirements for later cap adjustments."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    minimum_samples: PositiveStrictInt
    warning_percentile: Annotated[StrictInt, Field(ge=1, le=100)]
    soft_generation_headroom: PositiveStrictInt
    hard_generation_headroom: PositiveStrictInt


class StagePolicy(BaseModel):
    """One independently accounted leaf in the cache tree."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    path: Path
    warning_bytes: PositiveStrictInt
    soft_bytes: PositiveStrictInt
    hard_bytes: PositiveStrictInt
    prune: PruneMethod
    maximum_age_hours: PositiveStrictInt
    maximum_count: PositiveStrictInt | None = None
    external: StrictBool = False

    @model_validator(mode="after")
    def validate_stage(self) -> StagePolicy:
        object.__setattr__(self, "path", _relative_descendant(self.path, field="stage path"))
        if not self.warning_bytes <= self.soft_bytes <= self.hard_bytes:
            raise ValueError("stage limits must satisfy warning_bytes <= soft_bytes <= hard_bytes")
        if self.external != (self.prune is PruneMethod.EXTERNAL):
            raise ValueError("external stages must use the external prune method")
        return self


class CachePolicy(BaseModel):
    """The complete validated cache configuration."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    version: Annotated[StrictInt, Field(ge=1)]
    root: Path
    minimum_free_bytes: PositiveStrictInt
    calibration: CalibrationPolicy = CalibrationPolicy(
        minimum_samples=5,
        warning_percentile=95,
        soft_generation_headroom=1,
        hard_generation_headroom=1,
    )
    stages: dict[str, StagePolicy]

    @model_validator(mode="after")
    def validate_policy(self) -> CachePolicy:
        object.__setattr__(self, "root", _relative_descendant(self.root, field="cache root"))
        if self.root != Path("cache"):
            raise ValueError("cache root must be the repository cache directory")
        if not self.stages:
            raise ValueError("cache policy must declare at least one stage")
        for stage_id in self.stages:
            if not STAGE_ID.fullmatch(stage_id):
                raise ValueError(f"invalid cache stage id: {stage_id!r}")
        items = sorted(self.stages.items())
        for index, (left_id, left) in enumerate(items):
            for right_id, right in items[index + 1 :]:
                if left.path == right.path or left.path in right.path.parents or right.path in left.path.parents:
                    raise ValueError(
                        f"cache stage paths overlap: {left_id}={left.path} and "
                        f"{right_id}={right.path}"
                    )
        return self
