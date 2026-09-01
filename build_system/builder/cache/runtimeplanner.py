"""Pure retention planning for Docker, BuildKit, and Tart resources."""

from __future__ import annotations

from collections import defaultdict

from .models import CachePolicy
from .runtimemodels import (
    DockerRuntimePolicy,
    ResourceKind,
    RuntimeInventory,
    RuntimeOperation,
    RuntimePruneAction,
    RuntimePrunePlan,
    RuntimeResource,
    RuntimeSnapshot,
    TartRuntimePolicy,
)

NANOSECONDS_PER_HOUR = 3_600_000_000_000


def _repository(name: str) -> str:
    without_digest = name.split("@", 1)[0]
    return (
        without_digest.rsplit(":", 1)[0]
        if without_digest.rfind(":") > without_digest.rfind("/")
        else without_digest
    )


def _docker_actions(
    inventory: RuntimeInventory, policy: DockerRuntimePolicy
) -> tuple[RuntimePruneAction, ...]:
    actions: list[RuntimePruneAction] = []
    groups: dict[str, list[RuntimeResource]] = defaultdict(list)
    for resource in inventory.resources:
        if resource.kind is ResourceKind.IMAGE:
            groups[_repository(resource.names[0])].append(resource)
        elif resource.kind is ResourceKind.CONTAINER:
            expired = (
                resource.created_ns > 0
                and inventory.generated_ns
                >= resource.created_ns + policy.maximum_age_hours * NANOSECONDS_PER_HOUR
            )
            if resource.owned and expired and not resource.protected:
                actions.append(
                    RuntimePruneAction(
                        runtime_id=inventory.runtime_id,
                        operation=RuntimeOperation.REMOVE_CONTAINER,
                        target=resource.identity,
                        logical_bytes=resource.logical_bytes,
                        reason=f"stopped owned container older than {policy.maximum_age_hours}h",
                    )
                )
        elif (
            resource.kind is ResourceKind.BUILD_CACHE
            and resource.logical_bytes > policy.build_cache_keep_bytes
        ):
            actions.append(
                RuntimePruneAction(
                    runtime_id=inventory.runtime_id,
                    operation=RuntimeOperation.PRUNE_BUILD_CACHE,
                    target=resource.identity,
                    logical_bytes=resource.logical_bytes - policy.build_cache_keep_bytes,
                    reason=(
                        f"retain {policy.build_cache_keep_bytes} bytes of hottest BuildKit data "
                        f"and evict entries older than {policy.maximum_age_hours}h"
                    ),
                )
            )
    for resources in groups.values():
        ordered = sorted(resources, key=lambda item: (item.created_ns, item.identity), reverse=True)
        for position, resource in enumerate(ordered):
            if resource.protected or position < policy.keep_image_generations:
                continue
            actions.append(
                RuntimePruneAction(
                    runtime_id=inventory.runtime_id,
                    operation=RuntimeOperation.REMOVE_IMAGE,
                    target=resource.names[0],
                    logical_bytes=resource.logical_bytes,
                    reason=(
                        f"superseded owned image generation; retain newest "
                        f"{policy.keep_image_generations} per repository"
                    ),
                )
            )
    return tuple(actions)


def _tart_actions(
    inventory: RuntimeInventory, policy: TartRuntimePolicy
) -> tuple[RuntimePruneAction, ...]:
    del policy
    return tuple(
        RuntimePruneAction(
            runtime_id=inventory.runtime_id,
            operation=RuntimeOperation.DELETE_VM,
            target=resource.identity,
            logical_bytes=resource.logical_bytes,
            reason="stopped Capsem-owned Tart working VM has no remaining consumer",
        )
        for resource in inventory.resources
        if resource.kind is ResourceKind.VM and resource.owned and not resource.protected
    )


def plan_runtime_prune(snapshot: RuntimeSnapshot, policy: CachePolicy) -> RuntimePrunePlan:
    actions: list[RuntimePruneAction] = []
    violations = []
    for inventory in snapshot.runtimes:
        runtime = policy.runtimes[inventory.runtime_id]
        if not inventory.available:
            violations.append(f"{inventory.runtime_id} unavailable: {inventory.error}")
        elif isinstance(runtime, DockerRuntimePolicy):
            actions.extend(_docker_actions(inventory, runtime))
        elif isinstance(runtime, TartRuntimePolicy):
            actions.extend(_tart_actions(inventory, runtime))
    selected = tuple(sorted(actions, key=lambda item: (item.runtime_id, item.operation, item.target)))
    return RuntimePrunePlan(
        generated_ns=snapshot.generated_ns,
        reclaim_bytes=sum(action.logical_bytes for action in selected),
        actions=selected,
        violations=tuple(violations),
    )
