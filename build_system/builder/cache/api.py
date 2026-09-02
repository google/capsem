"""Typed public requests and results for every cache mechanism."""

from __future__ import annotations

from enum import StrEnum
from typing import Protocol

from pydantic import BaseModel, ConfigDict, StrictBool, StrictInt, StrictStr, model_validator

from .contract import CacheContract
from .stats import CacheUsage


class CacheOperation(StrEnum):
    """Mutations supported uniformly by disk and native-runtime caches."""

    PRUNE = "prune"
    ENFORCE = "enforce"
    CLEAN = "clean"


class CacheRequest(BaseModel):
    """One mechanism-independent cache mutation request."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    operation: CacheOperation
    cache_id: StrictStr
    apply: StrictBool
    reason: StrictStr

    @model_validator(mode="after")
    def values_are_explicit(self) -> CacheRequest:
        if not self.cache_id.strip():
            raise ValueError("cache_id must contain visible text")
        if self.apply and not self.reason.strip():
            raise ValueError("applied cache mutations require a reason")
        if self.operation is CacheOperation.ENFORCE and not self.apply:
            raise ValueError("enforcement is always applied")
        return self


class CacheMutationResult(BaseModel):
    """Uniform outcome returned without exposing an owner's storage mechanism."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    cache_id: StrictStr
    operation: CacheOperation
    before_size_bytes: StrictInt
    after_size_bytes: StrictInt
    reclaim_bytes: StrictInt
    action_count: StrictInt
    applied: StrictBool
    violations: tuple[StrictStr, ...]


class CacheBackend(Protocol):
    """Private backend seam implemented by disk, Docker/Colima, and Tart."""

    @property
    def cache_ids(self) -> frozenset[str]: ...

    def contract(self, cache_id: str) -> CacheContract: ...

    def usages(self) -> tuple[CacheUsage, ...]: ...

    def mutate(self, request: CacheRequest) -> CacheMutationResult: ...
