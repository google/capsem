"""Typed usage report for every configured cache owner."""

from __future__ import annotations

from enum import StrEnum

from pydantic import BaseModel, ConfigDict, StrictBool, StrictInt, StrictStr

from .contract import CacheScope, PruneStrategy
from .dockerimages import image_cache_size
from .models import CacheInventory, CachePolicy
from .render import bytes_label


class UsageState(StrEnum):
    OK = "ok"
    ABOVE_WARM = "above-warm"
    ABOVE_MAX = "above-max"
    UNAVAILABLE = "unavailable"


class CacheUsage(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)

    cache_id: StrictStr
    description: StrictStr
    scope: CacheScope
    current_size_bytes: StrictInt
    warm_size_bytes: StrictInt
    max_size_bytes: StrictInt
    prune_strategy: PruneStrategy
    state: UsageState


class CacheStats(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)

    healthy: StrictBool
    disk_size_bytes: StrictInt
    runtime_size_bytes: StrictInt
    caches: tuple[CacheUsage, ...]
    violations: tuple[StrictStr, ...]


def _state(size: int, warm: int, maximum: int) -> UsageState:
    if size > maximum:
        return UsageState.ABOVE_MAX
    if size > warm:
        return UsageState.ABOVE_WARM
    return UsageState.OK


def build_stats(
    inventory: CacheInventory,
    policy: CachePolicy,
    *,
    unavailable_is_violation: bool = True,
) -> CacheStats:
    """Join inventory with its authority so stats explain every byte limit."""
    caches = []
    violations = []
    for stage in inventory.stages:
        contract = policy.stages[stage.stage_id]
        state = _state(stage.logical_bytes, contract.warm_size_bytes, contract.max_size_bytes)
        if state is UsageState.ABOVE_MAX:
            violations.append(
                f"{stage.stage_id} uses {stage.logical_bytes} bytes above max size "
                f"{contract.max_size_bytes}"
            )
        caches.append(
            CacheUsage(
                cache_id=stage.stage_id,
                description=contract.description,
                scope=contract.scope,
                current_size_bytes=stage.logical_bytes,
                warm_size_bytes=contract.warm_size_bytes,
                max_size_bytes=contract.max_size_bytes,
                prune_strategy=contract.prune_strategy,
                state=state,
            )
        )
    for runtime in inventory.runtimes:
        contract = policy.runtimes[runtime.runtime_id]
        state = (
            _state(runtime.owned_bytes, contract.warm_size_bytes, contract.max_size_bytes)
            if runtime.available
            else UsageState.UNAVAILABLE
        )
        if state is UsageState.ABOVE_MAX:
            violations.append(
                f"{runtime.runtime_id} uses {runtime.owned_bytes} owned bytes above max size "
                f"{contract.max_size_bytes}"
            )
        elif state is UsageState.UNAVAILABLE and contract.required and unavailable_is_violation:
            violations.append(f"{runtime.runtime_id} unavailable: {runtime.error}")
        caches.append(
            CacheUsage(
                cache_id=runtime.runtime_id,
                description=contract.description,
                scope=contract.scope,
                current_size_bytes=runtime.owned_bytes,
                warm_size_bytes=contract.warm_size_bytes,
                max_size_bytes=contract.max_size_bytes,
                prune_strategy=contract.prune_strategy,
                state=state,
            )
        )
        if policy.control is not None and runtime.runtime_id == policy.control.docker.runtime_id:
            for cache_id, image in sorted(policy.control.docker.images.items()):
                size = image_cache_size(runtime, image)
                image_state = (
                    _state(size, image.warm_size_bytes, image.max_size_bytes)
                    if runtime.available
                    else UsageState.UNAVAILABLE
                )
                if image_state is UsageState.ABOVE_MAX:
                    violations.append(
                        f"{cache_id} uses {size} bytes above max size {image.max_size_bytes}"
                    )
                caches.append(
                    CacheUsage(
                        cache_id=cache_id,
                        description=image.description,
                        scope=image.scope,
                        current_size_bytes=size,
                        warm_size_bytes=image.warm_size_bytes,
                        max_size_bytes=image.max_size_bytes,
                        prune_strategy=image.prune_strategy,
                        state=image_state,
                    )
                )
    return CacheStats(
        healthy=not violations,
        disk_size_bytes=inventory.logical_bytes,
        runtime_size_bytes=sum(runtime.owned_bytes for runtime in inventory.runtimes),
        caches=tuple(caches),
        violations=tuple(violations),
    )


def render(report: CacheStats) -> str:
    lines = [
        f"Cache stats: {'OK' if report.healthy else 'ACTION REQUIRED'}",
        f"Disk: {bytes_label(report.disk_size_bytes)}; "
        f"native runtimes: {bytes_label(report.runtime_size_bytes)}",
    ]
    lines.extend(
        f"  {cache.cache_id}: {cache.description}\n"
        f"    {bytes_label(cache.current_size_bytes)} / "
        f"warm {bytes_label(cache.warm_size_bytes)} / max "
        f"{bytes_label(cache.max_size_bytes)} [{cache.scope}, {cache.prune_strategy}]"
        for cache in report.caches
    )
    lines.extend(f"  VIOLATION: {violation}" for violation in report.violations)
    return "\n".join(lines)
