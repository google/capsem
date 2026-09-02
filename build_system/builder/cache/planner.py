"""Pure deterministic retention and clean planning."""

from __future__ import annotations

from .contract import PruneStrategy
from .inventorymodels import RetentionInventory
from .models import CacheEntry, CacheInventory, CachePolicy, PruneAction, PrunePlan

NANOSECONDS_PER_HOUR = 3_600_000_000_000


def _retention_key(strategy: PruneStrategy, entry: CacheEntry) -> tuple[int, int, str]:
    """Order by the semantic clock named by the configured strategy."""
    if strategy is PruneStrategy.LRU:
        return entry.last_used_ns, entry.created_ns, entry.key
    return entry.created_ns, entry.last_used_ns, entry.key


def _age_clock(strategy: PruneStrategy, entry: CacheEntry) -> int:
    return entry.last_used_ns if strategy is PruneStrategy.LRU else entry.created_ns


def plan_prune(inventory: CacheInventory | RetentionInventory, policy: CachePolicy) -> PrunePlan:
    """Select expired, surplus, and pressure candidates without touching pinned state."""
    actions: list[PruneAction] = []
    violations: list[str] = []
    selected: dict[str, set[str]] = {stage.stage_id: set() for stage in inventory.stages}

    def choose(stage, entry, reason: str) -> None:
        actions.append(
            PruneAction(
                stage_id=stage.stage_id,
                key=entry.key,
                path=stage.path / entry.relative_path,
                logical_bytes=entry.logical_bytes,
                reason=reason,
            )
        )
        selected[stage.stage_id].add(entry.key)

    for stage in inventory.stages:
        stage_policy = policy.stages[stage.stage_id]
        remaining = stage.logical_bytes
        remaining_count = sum(entry.managed for entry in stage.entries)
        if stage_policy.prune_strategy is PruneStrategy.NONE:
            if remaining > stage_policy.max_size_bytes:
                violations.append(
                    f"{stage.stage_id} uses {remaining} bytes above max size "
                    f"{stage_policy.max_size_bytes}"
                )
            if (
                stage_policy.maximum_count is not None
                and remaining_count > stage_policy.maximum_count
            ):
                violations.append(
                    f"{stage.stage_id} remains at {remaining_count} entries above count cap "
                    f"{stage_policy.maximum_count}"
                )
            continue
        ordered = sorted(
            (entry for entry in stage.entries if entry.managed),
            key=lambda entry: _retention_key(stage_policy.prune_strategy, entry),
        )
        maximum_age = stage_policy.maximum_age_hours * NANOSECONDS_PER_HOUR
        over_max = remaining > stage_policy.max_size_bytes
        for entry in ordered:
            expired = (
                inventory.generated_ns
                >= _age_clock(stage_policy.prune_strategy, entry) + maximum_age
            )
            recover_to_warm = over_max and remaining > stage_policy.warm_size_bytes
            over_count = (
                stage_policy.maximum_count is not None
                and remaining_count > stage_policy.maximum_count
            )
            if entry.protected or not (expired or recover_to_warm or over_count):
                continue
            reason = (
                "expired"
                if expired
                else "over max size; recover to warm size"
                if recover_to_warm
                else "over count cap"
            )
            choose(stage, entry, reason)
            remaining -= entry.logical_bytes
            remaining_count -= 1
        if remaining > stage_policy.max_size_bytes:
            violations.append(
                f"{stage.stage_id} remains {remaining} bytes above max size "
                f"{stage_policy.max_size_bytes}"
            )
        if stage_policy.maximum_count is not None and remaining_count > stage_policy.maximum_count:
            violations.append(
                f"{stage.stage_id} remains at {remaining_count} entries above count cap "
                f"{stage_policy.maximum_count}"
            )
    return PrunePlan(
        generated_ns=inventory.generated_ns,
        reclaim_bytes=sum(action.logical_bytes for action in actions),
        actions=tuple(actions),
        violations=tuple(violations),
    )


def plan_clean(inventory: CacheInventory, stage_id: str) -> PrunePlan:
    """Select every unprotected entry in one stage or the complete cache."""
    selected = (
        inventory.stages
        if stage_id == "all"
        else tuple(stage for stage in inventory.stages if stage.stage_id == stage_id)
    )
    if not selected:
        known = ", ".join(stage.stage_id for stage in inventory.stages)
        raise KeyError(f"unknown cache stage {stage_id!r}; expected one of: {known}, all")
    actions = tuple(
        PruneAction(
            stage_id=stage.stage_id,
            key=entry.key,
            path=stage.path / entry.relative_path,
            logical_bytes=entry.logical_bytes,
            reason="explicit clean",
        )
        for stage in selected
        for entry in stage.entries
        if entry.managed and not entry.protected
    )
    return PrunePlan(
        generated_ns=inventory.generated_ns,
        reclaim_bytes=sum(action.logical_bytes for action in actions),
        actions=actions,
        violations=(),
    )
