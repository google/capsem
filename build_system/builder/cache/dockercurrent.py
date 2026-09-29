"""The current generation of every Docker image a checkout's gate uses.

Retention used to guess "current" as the newest image by creation time. That
is not what a gate needs: a BuildKit cache hit reproduces an old image with
its old timestamp, one repository holds a current tag per profile
(`capsem-rootfs-dependencies-*` is keyed by profile), and two checkouts on
different sources each have their own. On 2026-09-29 routine enforcement
removed the current `capsem-kernel-dependencies-x86_64` generation of the
running release proof, which rebuilt it minutes later, and an early enforce
evicted the current host builder, whose rebuild blocked the proof for hours.

So the gate declares it. Whenever a gate makes an image available -- built or
already present -- it records the tag here for its checkout and slot. Prune
and enforcement never remove a live record's tag; a live record's checkout
still exists and its record is younger than the Docker runtime's age bound.
Recording a new tag for the same checkout and slot supersedes the old one, so
retention still retires what no current checkout names.
"""

from __future__ import annotations

import hashlib
import os
import time
from pathlib import Path
from typing import Literal

from pydantic import BaseModel, ConfigDict, StrictInt, StrictStr, ValidationError

from .dockerimages import repository_name
from .models import CachePolicy
from .paths import CachePaths
from .runtimemodels import DockerRuntimePolicy, ResourceKind, RuntimeSnapshot

NANOSECONDS_PER_HOUR = 3_600_000_000_000
DIRECTORY = "docker-current"


class CurrentGeneration(BaseModel):
    """One checkout's current tag for one repository slot."""

    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    schema_id: Literal["capsem.docker-current.v1"] = "capsem.docker-current.v1"
    repository: StrictStr
    tag: StrictStr
    checkout: StrictStr
    slot: StrictStr
    recorded_ns: StrictInt


def _directory(paths: CachePaths, policy: CachePolicy) -> Path:
    if policy.control is None:
        raise ValueError("cache policy has no Docker control to record current images in")
    return paths.stage(policy.control.docker.current_stage) / DIRECTORY


def record(
    paths: CachePaths,
    *,
    tag: str,
    checkout: Path,
    slot: str = "",
    now_ns: int | None = None,
) -> Path:
    """Declare `tag` the current generation of its repository for one checkout slot."""
    if not checkout.is_absolute():
        raise ValueError("a current image checkout must be absolute")
    policy = paths.policy
    repository = repository_name(tag)
    if repository == tag:
        raise ValueError(f"current image {tag!r} must name an exact tag")
    value = CurrentGeneration(
        repository=repository,
        tag=tag,
        checkout=str(checkout),
        slot=slot,
        recorded_ns=time.time_ns() if now_ns is None else now_ns,
    )
    key = hashlib.sha256(f"{repository}\0{checkout}\0{slot}".encode()).hexdigest()[:32]
    directory = _directory(paths, policy)
    directory.mkdir(parents=True, exist_ok=True)
    target = directory / f"{key}.json"
    staging = directory / f".{key}.{os.getpid()}.tmp"
    staging.write_text(value.model_dump_json(), encoding="utf-8")
    staging.replace(target)
    return target


def live_tags(paths: CachePaths, *, now_ns: int) -> frozenset[str]:
    """Tags some existing checkout declared current within the runtime age bound."""
    policy = paths.policy
    if policy.control is None:
        return frozenset()
    runtime = policy.runtimes.get(policy.control.docker.runtime_id)
    if not isinstance(runtime, DockerRuntimePolicy):
        return frozenset()
    oldest = now_ns - runtime.maximum_age_hours * NANOSECONDS_PER_HOUR
    tags = set()
    for path in sorted(_directory(paths, policy).glob("*.json")):
        try:
            value = CurrentGeneration.model_validate_json(path.read_bytes())
        except (OSError, ValidationError):
            continue
        if value.recorded_ns >= oldest and Path(value.checkout).is_dir():
            tags.add(value.tag)
    return frozenset(tags)


def mark(snapshot: RuntimeSnapshot, paths: CachePaths) -> RuntimeSnapshot:
    """Flag every image a live record names as current, for every planner."""
    tags = live_tags(paths, now_ns=snapshot.generated_ns)
    if not tags:
        return snapshot
    runtimes = tuple(
        inventory.model_copy(
            update={
                "resources": tuple(
                    item.model_copy(update={"current": True})
                    if item.kind is ResourceKind.IMAGE and tags.intersection(item.names)
                    else item
                    for item in inventory.resources
                )
            }
        )
        for inventory in snapshot.runtimes
    )
    return snapshot.model_copy(update={"runtimes": runtimes})
