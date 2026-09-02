from __future__ import annotations

from .dockerimages import plan_docker_images
from .models import CachePolicy
from .runtimemodels import (
    DockerRuntimePolicy,
    ResourceKind,
    RuntimeInventory,
    RuntimeOperation,
    RuntimePruneAction,
    RuntimePrunePlan,
    RuntimeSnapshot,
    TartRuntimePolicy,
)

NANOSECONDS_PER_HOUR = 3_600_000_000_000


def _docker_actions(
    inventory: RuntimeInventory, policy: DockerRuntimePolicy, cache_policy: CachePolicy
) -> tuple[tuple[RuntimePruneAction, ...], tuple[str, ...]]:
    actions: list[RuntimePruneAction] = []
    for resource in inventory.resources:
        if resource.kind is ResourceKind.CONTAINER:
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
    image_actions, image_violations = plan_docker_images(
        inventory, cache_policy, default_keep=policy.keep_image_generations
    )
    actions.extend(image_actions)
    selected = {action.target for action in actions}
    recovered = sum(action.logical_bytes for action in actions)
    projected = max(0, inventory.owned_bytes - recovered)
    if inventory.owned_bytes > policy.max_size_bytes and projected > policy.warm_size_bytes:
        build = next(
            (item for item in inventory.resources if item.kind is ResourceKind.BUILD_CACHE), None
        )
        if build is not None and build.identity not in selected:
            reclaim = min(build.logical_bytes, projected - policy.warm_size_bytes)
            actions.append(
                RuntimePruneAction(
                    runtime_id=inventory.runtime_id,
                    operation=RuntimeOperation.PRUNE_BUILD_CACHE,
                    target=build.identity,
                    logical_bytes=reclaim,
                    reason=(
                        f"Docker cache exceeded max size {policy.max_size_bytes}; "
                        f"recover to warm size {policy.warm_size_bytes}"
                    ),
                    keep_bytes=max(0, build.logical_bytes - reclaim),
                    all_unused=True,
                )
            )
            projected -= reclaim
    violations = [*image_violations]
    if projected > policy.max_size_bytes:
        violations.append(
            f"{inventory.runtime_id} remains {projected} owned bytes above max size "
            f"{policy.max_size_bytes}"
        )
    return tuple(actions), tuple(violations)


def _tart_actions(
    inventory: RuntimeInventory, policy: TartRuntimePolicy
) -> tuple[tuple[RuntimePruneAction, ...], tuple[str, ...]]:
    actions = tuple(
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
    projected = max(0, inventory.owned_bytes - sum(item.logical_bytes for item in actions))
    violations = (
        (
            f"{inventory.runtime_id} remains {projected} owned bytes above max size {policy.max_size_bytes}",
        )
        if projected > policy.max_size_bytes
        else ()
    )
    return actions, violations


def plan_runtime_prune(snapshot: RuntimeSnapshot, policy: CachePolicy) -> RuntimePrunePlan:
    actions: list[RuntimePruneAction] = []
    violations = []
    for inventory in snapshot.runtimes:
        runtime = policy.runtimes[inventory.runtime_id]
        if not inventory.available:
            if runtime.required:
                violations.append(f"{inventory.runtime_id} unavailable: {inventory.error}")
        elif isinstance(runtime, DockerRuntimePolicy):
            selected, errors = _docker_actions(inventory, runtime, policy)
            actions.extend(selected)
            violations.extend(errors)
        elif isinstance(runtime, TartRuntimePolicy):
            selected, errors = _tart_actions(inventory, runtime)
            actions.extend(selected)
            violations.extend(errors)
    actions.sort(key=lambda item: (item.runtime_id, item.operation, item.target))
    selected = tuple(actions)
    return RuntimePrunePlan(
        generated_ns=snapshot.generated_ns,
        reclaim_bytes=sum(action.logical_bytes for action in selected),
        actions=selected,
        violations=tuple(violations),
    )


def plan_runtime_clean(snapshot: RuntimeSnapshot, policy: CachePolicy) -> RuntimePrunePlan:
    actions, violations = [], []
    for inventory in snapshot.runtimes:
        runtime = policy.runtimes[inventory.runtime_id]
        if not inventory.available:
            if runtime.required:
                violations.append(f"{inventory.runtime_id} unavailable: {inventory.error}")
            continue
        for item in inventory.resources:
            if not item.owned or item.protected:
                continue
            if item.kind is ResourceKind.IMAGE:
                operation = RuntimeOperation.REMOVE_IMAGE
                targets = item.names
            elif item.kind is ResourceKind.CONTAINER:
                operation = RuntimeOperation.REMOVE_CONTAINER
                targets = (item.identity,)
            elif item.kind is ResourceKind.BUILD_CACHE:
                operation = RuntimeOperation.CLEAR_BUILD_CACHE
                targets = (item.identity,)
            elif item.kind is ResourceKind.VM:
                operation = RuntimeOperation.DELETE_VM
                targets = (item.identity,)
            else:  # pragma: no cover - enum is exhaustive
                continue
            actions.extend(
                RuntimePruneAction(
                    runtime_id=inventory.runtime_id,
                    operation=operation,
                    target=target,
                    logical_bytes=item.logical_bytes,
                    reason="explicit cold cleanup of an inactive Capsem-owned cache",
                )
                for target in targets
            )
    values = tuple(sorted(actions, key=lambda item: (item.runtime_id, item.operation, item.target)))
    return RuntimePrunePlan(
        generated_ns=snapshot.generated_ns,
        reclaim_bytes=sum(item.logical_bytes for item in values),
        actions=values,
        violations=tuple(violations),
    )
