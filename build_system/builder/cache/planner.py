"""Pure deterministic retention and clean planning."""

from __future__ import annotations

from .inventorymodels import RetentionInventory
from .models import CacheInventory, CachePolicy, PruneAction, PruneMethod, PrunePlan

NANOSECONDS_PER_HOUR = 3_600_000_000_000


def plan_prune(inventory: CacheInventory | RetentionInventory, policy: CachePolicy) -> PrunePlan:
    """Select expired, surplus, and pressure candidates without touching pinned state."""
    actions: list[PruneAction] = []
    violations: list[str] = []
    selected: dict[str, set[str]] = {stage.stage_id: set() for stage in inventory.stages}
    recovered_allocated = 0

    def choose(stage, entry, reason: str) -> None:
        nonlocal recovered_allocated
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
        recovered_allocated += entry.allocated_bytes

    for stage in inventory.stages:
        stage_policy = policy.stages[stage.stage_id]
        remaining = stage.logical_bytes
        remaining_count = sum(entry.managed for entry in stage.entries)
        if stage_policy.prune in {PruneMethod.NONE, PruneMethod.EXTERNAL}:
            if remaining > stage_policy.soft_bytes:
                violations.append(
                    f"{stage.stage_id} remains {remaining} bytes above soft cap "
                    f"{stage_policy.soft_bytes}"
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
            key=lambda entry: (entry.last_used_ns, entry.key),
        )
        maximum_age = stage_policy.maximum_age_hours * NANOSECONDS_PER_HOUR
        for entry in ordered:
            expired = inventory.generated_ns >= entry.created_ns + maximum_age
            over_cap = remaining > stage_policy.soft_bytes
            over_count = (
                stage_policy.maximum_count is not None
                and remaining_count > stage_policy.maximum_count
            )
            if entry.protected or not (expired or over_cap or over_count):
                continue
            reason = "expired" if expired else "over soft cap" if over_cap else "over count cap"
            choose(stage, entry, reason)
            remaining -= entry.logical_bytes
            remaining_count -= 1
        if remaining > stage_policy.soft_bytes:
            violations.append(
                f"{stage.stage_id} remains {remaining} bytes above soft cap "
                f"{stage_policy.soft_bytes}"
            )
        if stage_policy.maximum_count is not None and remaining_count > stage_policy.maximum_count:
            violations.append(
                f"{stage.stage_id} remains at {remaining_count} entries above count cap "
                f"{stage_policy.maximum_count}"
            )

    required = policy.minimum_free_bytes - (inventory.filesystem_free_bytes + recovered_allocated)
    if required > 0:
        candidates = sorted(
            (
                (stage, entry)
                for stage in inventory.stages
                if policy.stages[stage.stage_id].prune
                not in {PruneMethod.NONE, PruneMethod.EXTERNAL}
                for entry in stage.entries
                if entry.managed
                and not entry.protected
                and entry.key not in selected[stage.stage_id]
                and entry.allocated_bytes > 0
            ),
            key=lambda item: (item[1].last_used_ns, item[0].stage_id, item[1].key),
        )
        for stage, entry in candidates:
            choose(stage, entry, "below free-space reserve")
            required -= entry.allocated_bytes
            if required <= 0:
                break
    projected_free = inventory.filesystem_free_bytes + recovered_allocated
    if projected_free < policy.minimum_free_bytes:
        violations.append(
            f"filesystem remains at {projected_free} bytes below free-space reserve "
            f"{policy.minimum_free_bytes}"
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
