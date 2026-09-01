"""Deterministic retention decisions for reusable build products."""

from __future__ import annotations

from typing import Annotated

from pydantic import BaseModel, ConfigDict, Field, StrictBool, StrictInt, model_validator


class CacheLimits(BaseModel):
    """The three independent ways a retained cache is bounded."""

    model_config = ConfigDict(extra="forbid", frozen=True, strict=True, allow_inf_nan=False)

    maximum_count: Annotated[StrictInt, Field(gt=0)]
    maximum_age_seconds: Annotated[float, Field(gt=0)]
    maximum_bytes: Annotated[StrictInt, Field(gt=0)]


class CacheProduct(BaseModel):
    """One product ordered by observed use, with a stable tie breaker."""

    model_config = ConfigDict(extra="forbid", frozen=True, strict=True, allow_inf_nan=False)

    key: Annotated[str, Field(min_length=1)]
    size_bytes: Annotated[StrictInt, Field(ge=0)]
    created_at: Annotated[float, Field(ge=0)]
    last_used_at: Annotated[float, Field(ge=0)]
    protected: StrictBool = False

    @model_validator(mode="after")
    def timestamps_are_ordered(self) -> CacheProduct:
        if self.last_used_at < self.created_at:
            raise ValueError("cache product last use cannot precede creation")
        return self


class ReclaimPlan(BaseModel):
    """What may leave and whether pinned products still violate policy."""

    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    evict: tuple[str, ...]
    violations: tuple[str, ...]


def plan_reclaim(
    products: tuple[CacheProduct, ...], limits: CacheLimits, *, now: float
) -> ReclaimPlan:
    """Evict expired then least-recently-used unpinned products deterministically."""
    if now < 0:
        raise ValueError("cache policy clock cannot be negative")
    remaining = {product.key: product for product in products}
    if len(remaining) != len(products):
        raise ValueError("cache product keys must be unique")
    ordered = sorted(products, key=lambda item: (item.last_used_at, item.key))
    evicted: list[str] = []

    for product in ordered:
        invalid_clock = product.created_at > now or product.last_used_at > now
        if not product.protected and (
            invalid_clock or now - product.created_at > limits.maximum_age_seconds
        ):
            remaining.pop(product.key)
            evicted.append(product.key)

    for product in ordered:
        if product.key not in remaining or product.protected:
            continue
        total = sum(item.size_bytes for item in remaining.values())
        if len(remaining) <= limits.maximum_count and total <= limits.maximum_bytes:
            break
        remaining.pop(product.key)
        evicted.append(product.key)

    total = sum(item.size_bytes for item in remaining.values())
    violations: list[str] = []
    if len(remaining) > limits.maximum_count:
        violations.append(f"count {len(remaining)} exceeds {limits.maximum_count}")
    if total > limits.maximum_bytes:
        violations.append(f"bytes {total} exceeds {limits.maximum_bytes}")
    expired = sorted(
        item.key
        for item in remaining.values()
        if now - item.created_at > limits.maximum_age_seconds
    )
    if expired:
        violations.append("expired protected products: " + ", ".join(expired))
    future = sorted(
        item.key
        for item in remaining.values()
        if item.created_at > now or item.last_used_at > now
    )
    if future:
        violations.append("future-dated protected products: " + ", ".join(future))
    return ReclaimPlan(evict=tuple(evicted), violations=tuple(violations))
