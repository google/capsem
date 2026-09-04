"""Enforce owned cache sizes through the journaled mutation boundaries."""

from __future__ import annotations

from pydantic import BaseModel, ConfigDict, StrictBool, StrictInt, StrictStr

from .inventory import scan_inventory, select_inventory
from .models import CachePolicy
from .operations import apply_prune
from .paths import CachePaths
from .planner import plan_prune
from .runtimeexec import CommandRunner, execute
from .runtimeinventory import scan_runtimes
from .runtimeoperations import apply_runtime_prune
from .runtimeplanner import plan_runtime_prune


class EnforcementResult(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    cache_id: StrictStr
    before_size_bytes: StrictInt
    after_size_bytes: StrictInt
    pruned: StrictBool
    reclaim_bytes: StrictInt
    action_count: StrictInt
    violations: tuple[StrictStr, ...]


def enforce_repository(
    paths: CachePaths, policy: CachePolicy, cache_id: str, *, reason: str
) -> EnforcementResult:
    """Prune one repository owner, or all owners, when a maximum is crossed."""
    inventory = select_inventory(scan_inventory(paths, policy), cache_id)
    plan = plan_prune(inventory, policy)
    if plan.actions:
        apply_prune(paths, plan, reason=reason)
    after = select_inventory(scan_inventory(paths, policy), cache_id)
    violations = plan_prune(after, policy).violations
    return EnforcementResult(
        cache_id=cache_id,
        before_size_bytes=inventory.logical_bytes,
        after_size_bytes=after.logical_bytes,
        pruned=bool(plan.actions),
        reclaim_bytes=plan.reclaim_bytes,
        action_count=len(plan.actions),
        violations=violations,
    )


def enforce_runtime(
    paths: CachePaths,
    policy: CachePolicy,
    runtime_id: str,
    *,
    reason: str,
    runner: CommandRunner = execute,
) -> EnforcementResult:
    """Prune one native runtime cache and prove its owned bytes are bounded."""
    if runtime_id not in policy.runtimes:
        raise ValueError(f"unknown runtime cache {runtime_id!r}")
    before_snapshot = scan_runtimes(policy, runner=runner, runtime_ids=frozenset({runtime_id}))
    before = before_snapshot.runtimes[0]
    plan = plan_runtime_prune(before_snapshot, policy)
    failures = []
    if plan.actions:
        applied = apply_runtime_prune(paths, policy, plan, reason=reason, runner=runner)
        failures.extend(item.output for item in applied.results if item.returncode != 0)
    after_snapshot = scan_runtimes(policy, runner=runner, runtime_ids=frozenset({runtime_id}))
    after = after_snapshot.runtimes[0]
    contract = policy.runtimes[runtime_id]
    violations = list(failures)
    if not after.available:
        if contract.required:
            violations.append(f"{runtime_id} unavailable: {after.error}")
    elif after.owned_bytes > contract.max_size_bytes:
        violations.append(
            f"{runtime_id} uses {after.owned_bytes} owned bytes above max size "
            f"{contract.max_size_bytes}"
        )
    return EnforcementResult(
        cache_id=runtime_id,
        before_size_bytes=before.owned_bytes,
        after_size_bytes=after.owned_bytes,
        pruned=bool(plan.actions),
        reclaim_bytes=plan.reclaim_bytes,
        action_count=len(plan.actions),
        violations=tuple(violations),
    )
