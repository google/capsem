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
    tag = without_digest.rfind(":")
    return without_digest[:tag] if tag > without_digest.rfind("/") else without_digest


def _docker_actions(
    inventory: RuntimeInventory, policy: DockerRuntimePolicy, cache_policy: CachePolicy
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
                    keep_bytes=policy.build_cache_keep_bytes,
                    maximum_age_hours=policy.maximum_age_hours,
                )
            )
    for repository, resources in groups.items():
        keep_generations = policy.keep_image_generations
        if cache_policy.control is not None:
            keep_generations = cache_policy.control.docker.image_generation_limit(
                repository, default=keep_generations
            )
        ordered = sorted(resources, key=lambda item: (item.created_ns, item.identity), reverse=True)
        for position, resource in enumerate(ordered):
            if resource.protected or position < keep_generations:
                continue
            actions.append(
                RuntimePruneAction(
                    runtime_id=inventory.runtime_id,
                    operation=RuntimeOperation.REMOVE_IMAGE,
                    target=resource.names[0],
                    logical_bytes=resource.logical_bytes,
                    reason=(
                        f"superseded owned image generation; retain newest "
                        f"{keep_generations} per repository"
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
            if runtime.required:
                violations.append(f"{inventory.runtime_id} unavailable: {inventory.error}")
        elif isinstance(runtime, DockerRuntimePolicy):
            actions.extend(_docker_actions(inventory, runtime, policy))
        elif isinstance(runtime, TartRuntimePolicy):
            actions.extend(_tart_actions(inventory, runtime))
    actions.sort(key=lambda item: (item.runtime_id, item.operation, item.target))
    selected = tuple(actions)
    return RuntimePrunePlan(
        generated_ns=snapshot.generated_ns,
        reclaim_bytes=sum(action.logical_bytes for action in selected),
        actions=selected,
        violations=tuple(violations),
    )


def _runtime(snapshot: RuntimeSnapshot, runtime_id: str) -> RuntimeInventory:
    try:
        return next(item for item in snapshot.runtimes if item.runtime_id == runtime_id)
    except StopIteration:
        raise ValueError(f"runtime snapshot omits {runtime_id!r}") from None


def plan_repository_reclaim(
    snapshot: RuntimeSnapshot,
    policy: CachePolicy,
    resource_id: str,
    *,
    keep: str,
    protect: tuple[str, ...] = (),
) -> RuntimePrunePlan:
    """Retire superseded tags around an exact caller-owned anchor."""
    if policy.control is None:
        raise ValueError("cache policy has no runtime control section")
    control = policy.control.docker
    try:
        image_policy = control.images[resource_id]
    except KeyError:
        raise ValueError(f"unknown image cache resource {resource_id!r}") from None
    if _repository(keep) != image_policy.repository:
        raise ValueError(f"{keep!r} is not a tag of {image_policy.repository!r}")
    invalid = sorted(tag for tag in protect if _repository(tag) != image_policy.repository)
    if invalid:
        raise ValueError(f"protected images are outside {image_policy.repository!r}: {invalid}")
    inventory = _runtime(snapshot, control.runtime_id)
    if not inventory.available:
        return RuntimePrunePlan(
            generated_ns=snapshot.generated_ns,
            reclaim_bytes=0,
            actions=(),
            violations=(f"{control.runtime_id} unavailable: {inventory.error}",),
        )
    resources = sorted(
        (
            item
            for item in inventory.resources
            if item.kind is ResourceKind.IMAGE
            and any(_repository(name) == image_policy.repository for name in item.names)
        ),
        key=lambda item: (item.created_ns, item.identity),
        reverse=True,
    )
    names = {name for item in resources for name in item.names}
    if keep not in names:
        raise ValueError(f"anchor image {keep!r} is absent; refusing unanchored reclaim")
    pinned = {keep, *protect}
    previous = 0
    actions = []
    for item in resources:
        repository_names = tuple(
            name for name in item.names if _repository(name) == image_policy.repository
        )
        if any(name in pinned for name in repository_names):
            continue
        if previous < image_policy.keep_previous:
            previous += 1
            continue
        if item.protected:
            continue
        actions.extend(
            RuntimePruneAction(
                runtime_id=control.runtime_id,
                operation=RuntimeOperation.REMOVE_IMAGE,
                target=name,
                logical_bytes=item.logical_bytes,
                reason=f"superseded generation of {image_policy.repository}",
            )
            for name in repository_names
        )
    selected = tuple(sorted(actions, key=lambda item: item.target))
    return RuntimePrunePlan(
        generated_ns=snapshot.generated_ns,
        reclaim_bytes=sum(item.logical_bytes for item in selected),
        actions=selected,
        violations=(),
    )


def plan_release(snapshot: RuntimeSnapshot, policy: CachePolicy, boundary: str) -> RuntimePrunePlan:
    """Release exact working images after their configured final consumer."""
    if policy.control is None:
        raise ValueError("cache policy has no runtime control section")
    control = policy.control.docker
    try:
        release = control.releases[boundary]
    except KeyError:
        raise ValueError(f"unknown cache release boundary {boundary!r}") from None
    inventory = _runtime(snapshot, control.runtime_id)
    actions = []
    violations = []
    for target in release.images:
        found = next(
            (
                item
                for item in inventory.resources
                if item.kind is ResourceKind.IMAGE and target in item.names
            ),
            None,
        )
        if found is None:
            continue
        if found.protected:
            violations.append(f"working image {target} still has an active container")
            continue
        actions.append(
            RuntimePruneAction(
                runtime_id=control.runtime_id,
                operation=RuntimeOperation.REMOVE_IMAGE,
                target=target,
                logical_bytes=found.logical_bytes,
                reason=f"final consumer completed at {boundary}",
            )
        )
    values = tuple(actions)
    return RuntimePrunePlan(
        generated_ns=snapshot.generated_ns,
        reclaim_bytes=sum(item.logical_bytes for item in values),
        actions=values,
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
