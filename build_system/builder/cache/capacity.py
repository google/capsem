"""Docker capacity enforcement through the journaled runtime boundary."""

from __future__ import annotations

import re
import time

from .controlmodels import CapacityDecision, DockerCapacitySnapshot
from .models import CachePolicy
from .paths import CachePaths
from .runtimeexec import CommandRunner, execute
from .runtimemodels import (
    DockerRuntimePolicy,
    RuntimeOperation,
    RuntimePruneAction,
    RuntimePrunePlan,
)
from .runtimeoperations import apply_runtime_prune


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


def ensure_capacity(
    paths: CachePaths,
    policy: CachePolicy,
    rail_id: str,
    *,
    reason: str,
    runner: CommandRunner = execute,
) -> CapacityDecision:
    """Prune only BuildKit when necessary, then prove the configured floor."""
    if policy.control is None:
        raise ValueError("cache policy has no runtime control section")
    control = policy.control.docker
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
    pruned = before.free_bytes < rail.minimum_free_bytes
    if pruned:
        plan = RuntimePrunePlan(
            generated_ns=time.time_ns(),
            reclaim_bytes=0,
            actions=(
                RuntimePruneAction(
                    runtime_id=control.runtime_id,
                    operation=RuntimeOperation.PRUNE_BUILD_CACHE,
                    target="buildkit",
                    logical_bytes=0,
                    reason=f"Docker free space is below the {rail_id} rail",
                    keep_bytes=rail.build_cache_keep_bytes,
                ),
            ),
            violations=(),
        )
        applied = apply_runtime_prune(paths, policy, plan, reason=reason, runner=runner)
        failures = tuple(
            f"BuildKit pressure prune failed: {item.output}"
            for item in applied.results
            if item.returncode != 0
        )
    else:
        failures = ()
    after = probe_capacity(policy, runner=runner) if pruned and not failures else before
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
