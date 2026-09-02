"""One capacity and retention contract shared by every cache owner."""

from __future__ import annotations

from enum import StrEnum
from typing import Annotated

from pydantic import BaseModel, ConfigDict, Field, StrictInt, StrictStr, model_validator

PositiveInt = Annotated[StrictInt, Field(gt=0)]


class CacheScope(StrEnum):
    """Where an owner stores and accounts for its bytes."""

    DISK = "disk"
    DOCKER = "docker"
    TART = "tart"


class PruneStrategy(StrEnum):
    """How an owner may select data after crossing its maximum size."""

    LRU = "lru"
    GENERATIONAL = "generational"
    EPHEMERAL = "ephemeral"
    NONE = "none"
    DOCKER = "docker"
    TART = "tart"


class CacheContract(BaseModel):
    """The uniform metadata and hysteresis limits required for every cache."""

    # TOML represents enum and path values as strings.  The fields remain
    # strict about scalar types while Pydantic performs those explicit schema
    # conversions at the configuration boundary.
    model_config = ConfigDict(extra="forbid", frozen=True)

    description: StrictStr
    scope: CacheScope
    max_size_bytes: PositiveInt
    warm_size_bytes: PositiveInt
    prune_strategy: PruneStrategy

    @classmethod
    def from_owner(cls, owner: CacheContract) -> CacheContract:
        """Project any backend-specific owner onto the common contract."""
        return cls.model_validate(owner.model_dump(include=set(cls.model_fields)))

    @model_validator(mode="after")
    def values_are_coherent(self) -> CacheContract:
        if not self.description.strip():
            raise ValueError("cache description must contain visible text")
        if self.warm_size_bytes > self.max_size_bytes:
            raise ValueError("cache warm_size_bytes cannot exceed max_size_bytes")
        return self
