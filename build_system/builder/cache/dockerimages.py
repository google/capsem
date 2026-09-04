"""Typed Docker image-cache planning behind the runtime backend."""

from __future__ import annotations

from collections import defaultdict

from .controlmodels import ImageCachePolicy
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
)

NANOSECONDS_PER_HOUR = 3_600_000_000_000


def repository_name(name: str) -> str:
    """Strip a Docker tag or digest while preserving a registry port."""
    without_digest = name.split("@", 1)[0]
    tag = without_digest.rfind(":")
    return without_digest[:tag] if tag > without_digest.rfind("/") else without_digest


def _runtime(snapshot: RuntimeSnapshot, runtime_id: str) -> RuntimeInventory:
    try:
        return next(item for item in snapshot.runtimes if item.runtime_id == runtime_id)
    except StopIteration:
        raise ValueError(f"runtime snapshot omits {runtime_id!r}") from None


def _groups(inventory: RuntimeInventory) -> dict[str, list[RuntimeResource]]:
    groups: dict[str, list[RuntimeResource]] = defaultdict(list)
    for resource in inventory.resources:
        if resource.kind is ResourceKind.IMAGE and resource.owned and resource.names:
            groups[repository_name(resource.names[0])].append(resource)
    return groups


def image_cache_size(inventory: RuntimeInventory, policy: ImageCachePolicy) -> int:
    """Return unique owned image bytes for one declared repository."""
    return sum(item.logical_bytes for item in _groups(inventory).get(policy.repository, ()))


def _policy_for_repository(
    policy: CachePolicy, repository: str
) -> tuple[str, ImageCachePolicy] | None:
    if policy.control is None:
        return None
    return next(
        (
            (cache_id, image)
            for cache_id, image in policy.control.docker.images.items()
            if image.repository == repository
        ),
        None,
    )


def _bounded_actions(
    inventory: RuntimeInventory,
    resources: list[RuntimeResource],
    policy: ImageCachePolicy,
    *,
    default_keep: int,
) -> tuple[list[RuntimePruneAction], list[str]]:
    ordered = sorted(resources, key=lambda item: (item.created_ns, item.identity), reverse=True)
    keep = max(1, policy.maximum_count or default_keep)
    initial = sum(item.logical_bytes for item in ordered)
    projected = initial
    over_max = initial > policy.max_size_bytes
    actions = []
    for position, resource in enumerate(ordered):
        expired = (
            policy.maximum_age_hours is not None
            and resource.created_ns > 0
            and inventory.generated_ns
            >= resource.created_ns + policy.maximum_age_hours * NANOSECONDS_PER_HOUR
        )
        pressure = over_max and projected > policy.warm_size_bytes
        over_count = position >= keep
        if resource.protected or position == 0 or not (expired or pressure or over_count):
            continue
        reason = (
            "expired Docker image generation"
            if expired
            else "image cache exceeded max size; recover to warm size"
            if pressure
            else f"superseded image generation; retain newest {keep}"
        )
        actions.append(
            RuntimePruneAction(
                runtime_id=inventory.runtime_id,
                operation=RuntimeOperation.REMOVE_IMAGE,
                target=resource.names[0],
                logical_bytes=resource.logical_bytes,
                reason=reason,
            )
        )
        projected -= resource.logical_bytes
    violations = (
        [
            f"Docker image cache {policy.repository} remains {projected} bytes above max "
            f"size {policy.max_size_bytes}"
        ]
        if projected > policy.max_size_bytes
        else []
    )
    return actions, violations


def plan_docker_images(
    inventory: RuntimeInventory, policy: CachePolicy, *, default_keep: int
) -> tuple[tuple[RuntimePruneAction, ...], tuple[str, ...]]:
    """Plan every owned repository, using declared child contracts where present."""
    actions, violations = [], []
    for repository, resources in _groups(inventory).items():
        declared = _policy_for_repository(policy, repository)
        if declared is not None:
            selected, errors = _bounded_actions(
                inventory, resources, declared[1], default_keep=default_keep
            )
            actions.extend(selected)
            violations.extend(errors)
            continue
        ordered = sorted(resources, key=lambda item: (item.created_ns, item.identity), reverse=True)
        actions.extend(
            RuntimePruneAction(
                runtime_id=inventory.runtime_id,
                operation=RuntimeOperation.REMOVE_IMAGE,
                target=resource.names[0],
                logical_bytes=resource.logical_bytes,
                reason=f"superseded owned image generation; retain newest {default_keep}",
            )
            for position, resource in enumerate(ordered)
            if position >= default_keep and not resource.protected
        )
    return tuple(actions), tuple(violations)


def plan_image_cache(
    snapshot: RuntimeSnapshot, policy: CachePolicy, cache_id: str
) -> RuntimePrunePlan:
    """Plan one declared Docker image owner by its common cache ID."""
    if policy.control is None or cache_id not in policy.control.docker.images:
        raise ValueError(f"unknown Docker image cache {cache_id!r}")
    control = policy.control.docker
    inventory = _runtime(snapshot, control.runtime_id)
    image = control.images[cache_id]
    runtime = policy.runtimes[control.runtime_id]
    if not isinstance(runtime, DockerRuntimePolicy):
        raise ValueError(f"image cache {cache_id!r} is not attached to a Docker runtime")
    resources = _groups(inventory).get(image.repository, [])
    actions, violations = _bounded_actions(
        inventory,
        resources,
        image,
        default_keep=runtime.keep_image_generations,
    )
    return RuntimePrunePlan(
        generated_ns=snapshot.generated_ns,
        reclaim_bytes=sum(item.logical_bytes for item in actions),
        actions=tuple(actions),
        violations=tuple(violations),
    )


def plan_image_clean(
    snapshot: RuntimeSnapshot, policy: CachePolicy, cache_id: str
) -> RuntimePrunePlan:
    """Plan explicit cleanup of one declared Docker image cache."""
    if policy.control is None or cache_id not in policy.control.docker.images:
        raise ValueError(f"unknown Docker image cache {cache_id!r}")
    control = policy.control.docker
    inventory = _runtime(snapshot, control.runtime_id)
    image = control.images[cache_id]
    actions = tuple(
        RuntimePruneAction(
            runtime_id=control.runtime_id,
            operation=RuntimeOperation.REMOVE_IMAGE,
            target=item.names[0],
            logical_bytes=item.logical_bytes,
            reason="explicit clean of an inactive Docker image cache",
        )
        for item in _groups(inventory).get(image.repository, ())
        if not item.protected
    )
    return RuntimePrunePlan(
        generated_ns=snapshot.generated_ns,
        reclaim_bytes=sum(item.logical_bytes for item in actions),
        actions=actions,
        violations=(),
    )


def plan_repository_reclaim(
    snapshot: RuntimeSnapshot,
    policy: CachePolicy,
    resource_id: str,
    *,
    keep: str,
    protect: tuple[str, ...] = (),
    anchor: RuntimeResource | None = None,
) -> RuntimePrunePlan:
    """Retire superseded tags around an exact caller-owned anchor."""
    if policy.control is None or resource_id not in policy.control.docker.images:
        raise ValueError(f"unknown image cache resource {resource_id!r}")
    control = policy.control.docker
    image = control.images[resource_id]
    if repository_name(keep) != image.repository:
        raise ValueError(f"{keep!r} is not a tag of {image.repository!r}")
    invalid = sorted(tag for tag in protect if repository_name(tag) != image.repository)
    if invalid:
        raise ValueError(f"protected images are outside {image.repository!r}: {invalid}")
    inventory = _runtime(snapshot, control.runtime_id)
    resources = sorted(
        _groups(inventory).get(image.repository, []),
        key=lambda item: (item.created_ns, item.identity),
        reverse=True,
    )
    names = {name for item in resources for name in item.names}
    anchor_names = () if anchor is None else anchor.names
    verified_anchor = (
        anchor is not None
        and anchor.kind is ResourceKind.IMAGE
        and anchor.owned
        and anchor.protected
        and keep in anchor_names
    )
    if keep not in names and not verified_anchor:
        raise ValueError(f"anchor image {keep!r} is absent; refusing unanchored reclaim")
    pinned = {keep, *protect}
    actions, previous = [], 0
    for item in resources:
        names = tuple(name for name in item.names if repository_name(name) == image.repository)
        if any(name in pinned for name in names) or item.protected:
            continue
        if previous < image.keep_previous:
            previous += 1
            continue
        actions.append(
            RuntimePruneAction(
                runtime_id=control.runtime_id,
                operation=RuntimeOperation.REMOVE_IMAGE,
                target=names[0],
                logical_bytes=item.logical_bytes,
                reason=f"superseded generation of {image.repository}",
            )
        )
    selected = tuple(sorted(actions, key=lambda item: item.target))
    return RuntimePrunePlan(
        generated_ns=snapshot.generated_ns,
        reclaim_bytes=sum(item.logical_bytes for item in selected),
        actions=selected,
        violations=(),
    )
