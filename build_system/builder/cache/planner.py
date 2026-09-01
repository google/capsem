"""Pure deterministic retention and clean planning."""

from __future__ import annotations

from .models import CacheInventory, CachePolicy, PruneAction, PruneMethod, PrunePlan

NANOSECONDS_PER_HOUR = 3_600_000_000_000


def plan_prune(inventory: CacheInventory, policy: CachePolicy) -> PrunePlan:
    """Select expired and least-recently-used entries down to every soft cap."""
    actions: list[PruneAction] = []
    violations: list[str] = []
    for stage in inventory.stages:
        stage_policy = policy.stages[stage.stage_id]
        remaining = stage.logical_bytes
        if stage_policy.prune in {PruneMethod.NONE, PruneMethod.EXTERNAL}:
            if remaining > stage_policy.soft_bytes:
                violations.append(
                    f"{stage.stage_id} remains {remaining} bytes above soft cap "
                    f"{stage_policy.soft_bytes}"
                )
            continue
        selected: set[str] = set()
        ordered = sorted(stage.entries, key=lambda entry: (entry.last_used_ns, entry.key))
        maximum_age = stage_policy.maximum_age_hours * NANOSECONDS_PER_HOUR
        for entry in ordered:
            expired = inventory.generated_ns >= entry.created_ns + maximum_age
            over_cap = remaining > stage_policy.soft_bytes
            if entry.protected or not (expired or over_cap):
                continue
            reason = "expired" if expired else "over soft cap"
            actions.append(
                PruneAction(
                    stage_id=stage.stage_id,
                    key=entry.key,
                    path=stage.path / entry.relative_path,
                    logical_bytes=entry.logical_bytes,
                    reason=reason,
                )
            )
            selected.add(entry.key)
            remaining -= entry.logical_bytes
        if remaining > stage_policy.soft_bytes:
            violations.append(
                f"{stage.stage_id} remains {remaining} bytes above soft cap "
                f"{stage_policy.soft_bytes}"
            )
        if selected and stage.external:
            violations.append(f"{stage.stage_id} requires its external runtime adapter")
    return PrunePlan(
        generated_ns=inventory.generated_ns,
        reclaim_bytes=sum(action.logical_bytes for action in actions),
        actions=tuple(actions),
        violations=tuple(violations),
    )


def plan_clean(inventory: CacheInventory, stage_id: str) -> PrunePlan:
    """Select every unprotected entry in one stage or the complete cache."""
    selected = inventory.stages if stage_id == "all" else tuple(
        stage for stage in inventory.stages if stage.stage_id == stage_id
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
        if not entry.protected
    )
    return PrunePlan(
        generated_ns=inventory.generated_ns,
        reclaim_bytes=sum(action.logical_bytes for action in actions),
        actions=actions,
        violations=(),
    )
