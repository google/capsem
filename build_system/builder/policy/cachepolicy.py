"""Deterministic retention decisions for reusable build products."""

from __future__ import annotations

import math
from dataclasses import dataclass


@dataclass(frozen=True)
class CacheLimits:
    """The three independent ways a retained cache is bounded."""

    maximum_count: int
    maximum_age_seconds: float
    maximum_bytes: int

    def __post_init__(self) -> None:
        if self.maximum_count <= 0:
            raise ValueError("cache maximum_count must be positive")
        if self.maximum_age_seconds <= 0:
            raise ValueError("cache maximum_age_seconds must be positive")
        if self.maximum_bytes <= 0:
            raise ValueError("cache maximum_bytes must be positive")


@dataclass(frozen=True)
class CacheProduct:
    """One product ordered by observed use, with a stable tie breaker."""

    key: str
    size_bytes: int
    created_at: float
    last_used_at: float
    protected: bool = False

    def __post_init__(self) -> None:
        if not self.key:
            raise ValueError("cache product key must be non-empty")
        if self.size_bytes < 0:
            raise ValueError("cache product size cannot be negative")
        if (
            not math.isfinite(self.created_at)
            or not math.isfinite(self.last_used_at)
            or self.created_at < 0
            or self.last_used_at < self.created_at
        ):
            raise ValueError("cache product timestamps are invalid")


@dataclass(frozen=True)
class ReclaimPlan:
    """What may leave and whether pinned products still violate policy."""

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
    return ReclaimPlan(tuple(evicted), tuple(violations))
