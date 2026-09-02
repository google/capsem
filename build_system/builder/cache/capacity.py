"""Docker capacity enforcement through the journaled runtime boundary."""

from __future__ import annotations

import re
import time

from .controlmodels import CapacityDecision, DockerCapacitySnapshot
from .dockeradapter import categories
from .models import CachePolicy
from .paths import CachePaths
from .runtimeexec import CommandRunner, execute
from .runtimeinventory import scan_runtimes
from .runtimemodels import (
    DockerRuntimePolicy,
    RuntimeOperation,
    RuntimePruneAction,
    RuntimePrunePlan,
)
from .runtimeoperations import apply_runtime_prune
from .runtimeplanner import plan_runtime_prune


def _docker(policy: CachePolicy) -> tuple[DockerRuntimePolicy, str]:
    if policy.control is None:
        raise ValueError("cache policy has no runtime control section")
    runtime_id = policy.control.docker.runtime_id
    runtime = policy.runtimes[runtime_id]
    if not isinstance(runtime, DockerRuntimePolicy):
        raise ValueError(f"capacity runtime {runtime_id!r} is not Docker")
    return runtime, runtime_id


def probe_capacity(
    policy: CachePolicy, *, runner: CommandRunner = execute
) -> DockerCapacitySnapshot:
    """Measure the daemon filesystem without interpreting host disk state."""
    runtime, _ = _docker(policy)
    assert policy.control is not None
    result = runner(
        (
            runtime.command,
            "run",
            "--rm",
            policy.control.docker.capacity_probe_image,
            "sh",
            "-c",
            "df -Pk / | awk 'NR == 2 { print $2, $3, $4 }'",
        ),
        runtime.timeout_seconds,
    )
    match = re.search(r"(?m)^(\d+)\s+(\d+)\s+(\d+)$", result.stdout)
    if result.returncode != 0 or match is None:
        error = "\n".join(part for part in (result.stdout, result.stderr) if part)
        return DockerCapacitySnapshot(available=False, error=error or "capacity probe failed")
    total, used, free = (int(value) * 1024 for value in match.groups())
    return DockerCapacitySnapshot(
        available=True,
        total_bytes=total,
        used_bytes=used,
        free_bytes=free,
    )


def _apply_retention(
    paths: CachePaths,
    policy: CachePolicy,
    runtime_id: str,
    *,
    reason: str,
    runner: CommandRunner,
) -> tuple[bool, tuple[str, ...]]:
    """Retire superseded owned resources before sacrificing reusable layers."""
    snapshot = scan_runtimes(
        policy,
        runner=runner,
        runtime_ids=frozenset({runtime_id}),
    )
    plan = plan_runtime_prune(snapshot, policy)
    if plan.violations:
        return False, tuple(
            f"Docker retention inventory failed: {item}" for item in plan.violations
        )
    retention = tuple(
        action
        for action in plan.actions
        if action.operation is not RuntimeOperation.PRUNE_BUILD_CACHE
    )
    if not retention:
        return False, ()
    plan = plan.model_copy(
        update={
            "actions": retention,
            "reclaim_bytes": sum(action.logical_bytes for action in retention),
        }
    )
    applied = apply_runtime_prune(paths, policy, plan, reason=reason, runner=runner)
    failures = tuple(
        f"Docker retention failed: {item.output}"
        for item in applied.results
        if item.returncode != 0
    )
    return True, failures


def ensure_capacity(
    paths: CachePaths,
    policy: CachePolicy,
    rail_id: str,
    *,
    reason: str,
    runner: CommandRunner = execute,
) -> CapacityDecision:
    """Retire superseded resources, then trim BuildKit and prove the floor."""
    if policy.control is None:
        raise ValueError("cache policy has no runtime control section")
    control = policy.control.docker
    runtime, _ = _docker(policy)
    try:
        rail = control.rails[rail_id]
    except KeyError:
        raise ValueError(f"unknown Docker capacity rail {rail_id!r}") from None
    before = probe_capacity(policy, runner=runner)
    if not before.available:
        return CapacityDecision(
            rail=rail_id,
            before=before,
            after=before,
            pruned=False,
            violations=(f"Docker capacity unavailable: {before.error}",),
        )
    pressured = before.free_bytes < rail.minimum_free_bytes
    pruned = False
    failures: tuple[str, ...] = ()
    after = before
    if pressured:
        pruned, failures = _apply_retention(
            paths,
            policy,
            control.runtime_id,
            reason=reason,
            runner=runner,
        )
        if pruned and not failures:
            after = probe_capacity(policy, runner=runner)
    keep_ceiling = rail.build_cache_keep_bytes
    attempts = 0
    target_free = rail.minimum_free_bytes + rail.reclaim_headroom_bytes
    while (
        pressured
        and not failures
        and after.available
        and after.free_bytes < target_free
        and attempts < rail.reclaim_attempts
    ):
        try:
            storage = categories(runtime, runner=runner)
            build_cache = next(row for row in storage if row.name == "Build Cache")
        except (StopIteration, ValueError) as error:
            return CapacityDecision(
                rail=rail_id,
                before=before,
                after=after,
                pruned=pruned,
                violations=(f"BuildKit capacity inventory failed: {error}",),
            )
        pressure = target_free - after.free_bytes
        reclaim_bytes = min(build_cache.reclaimable_bytes, pressure)
        if reclaim_bytes <= 0:
            break
        keep_bytes = max(0, min(keep_ceiling, build_cache.logical_bytes) - pressure)
        plan = RuntimePrunePlan(
            generated_ns=time.time_ns(),
            reclaim_bytes=reclaim_bytes,
            actions=(
                RuntimePruneAction(
                    runtime_id=control.runtime_id,
                    operation=RuntimeOperation.PRUNE_BUILD_CACHE,
                    target="buildkit",
                    logical_bytes=reclaim_bytes,
                    reason=(
                        f"Docker free space is below the {rail_id} rail; retain the hottest "
                        f"{keep_bytes} bytes and recover the configured headroom"
                    ),
                    keep_bytes=keep_bytes,
                    all_unused=True,
                ),
            ),
            violations=(),
        )
        applied = apply_runtime_prune(paths, policy, plan, reason=reason, runner=runner)
        failures = failures + tuple(
            f"BuildKit pressure prune failed: {item.output}"
            for item in applied.results
            if item.returncode != 0
        )
        pruned = True
        keep_ceiling = keep_bytes
        attempts += 1
        if not failures:
            after = probe_capacity(policy, runner=runner)
    violations = list(failures)
    if after.available and after.total_bytes < control.minimum_disk_bytes:
        violations.append(
            f"Docker disk is {after.total_bytes} bytes; policy minimum is "
            f"{control.minimum_disk_bytes} and recommendation is {control.recommended_disk_bytes}"
        )
    if not after.available or after.free_bytes < rail.minimum_free_bytes:
        violations.append(
            f"Docker rail {rail_id!r} requires {rail.minimum_free_bytes} free bytes; "
            f"{after.free_bytes} remain"
        )
    return CapacityDecision(
        rail=rail_id,
        before=before,
        after=after,
        pruned=pruned,
        violations=tuple(violations),
    )
