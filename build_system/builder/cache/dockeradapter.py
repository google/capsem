"""Docker image, container, and BuildKit cache inventory."""

from __future__ import annotations

import json
import time

from . import dockervolumes
from .dockerformat import json_lines, parse_size, reclaimable_size, timestamp
from .runtimeexec import CommandRunner, execute
from .runtimemodels import (
    DockerRuntimePolicy,
    ResourceKind,
    RuntimeCategory,
    RuntimeInventory,
    RuntimeKind,
    RuntimeResource,
)


def _categories(output: str) -> tuple[RuntimeCategory, ...]:
    rows = []
    for raw in json_lines(output):
        rows.append(
            RuntimeCategory(
                name=str(raw["Type"]),
                count=int(str(raw["TotalCount"])),
                active=int(str(raw["Active"])),
                logical_bytes=parse_size(str(raw["Size"])),
                reclaimable_bytes=reclaimable_size(str(raw["Reclaimable"])),
            )
        )
    return tuple(rows)


def categories(
    policy: DockerRuntimePolicy, *, runner: CommandRunner = execute
) -> tuple[RuntimeCategory, ...]:
    """Read Docker's typed top-level storage inventory without image traversal."""
    command = (policy.command, "system", "df", "--format", "{{json .}}")
    for attempt in range(policy.inventory_retry_attempts):
        summary = runner(command, policy.timeout_seconds)
        if summary.returncode == 0:
            break
        error = summary.stderr or summary.stdout or "Docker unavailable"
        transient = (
            "failed to calculate image disk usage" in error
            and "snapshot" in error
            and "does not exist" in error
        )
        if not transient or attempt + 1 == policy.inventory_retry_attempts:
            raise ValueError(error)
        time.sleep(policy.inventory_retry_delay_milliseconds / 1000)
    try:
        return _categories(summary.stdout)
    except (KeyError, TypeError, ValueError, json.JSONDecodeError) as error:
        raise ValueError(f"invalid Docker storage inventory: {error}") from error


def _owned_images(
    policy: DockerRuntimePolicy,
    runner: CommandRunner,
    used_images: set[str],
) -> tuple[RuntimeResource, ...]:
    listed = runner(
        (policy.command, "image", "ls", "--no-trunc", "--format", "{{json .}}"),
        policy.timeout_seconds,
    )
    if listed.returncode != 0:
        raise ValueError(listed.stderr or "Docker image inventory failed")
    ids = sorted(
        {
            str(row["ID"])
            for row in json_lines(listed.stdout)
            if any(str(row["Repository"]).startswith(prefix) for prefix in policy.image_prefixes)
        }
    )
    if not ids:
        return ()
    inspected = runner(
        (
            policy.command,
            "image",
            "inspect",
            "--format",
            "{{.Id}}\\t{{.Created}}\\t{{.Size}}\\t{{json .RepoTags}}",
            *ids,
        ),
        policy.timeout_seconds,
    )
    if inspected.returncode != 0:
        raise ValueError(inspected.stderr or "Docker image inspection failed")
    resources = []
    for line in inspected.stdout.splitlines():
        identity, created, size, encoded_names = line.split("\\t", 3)
        names = tuple(
            sorted(
                name
                for name in json.loads(encoded_names) or []
                if any(name.startswith(prefix) for prefix in policy.image_prefixes)
            )
        )
        if not names:
            continue
        created_at = timestamp(created)
        resources.append(
            RuntimeResource(
                kind=ResourceKind.IMAGE,
                identity=identity,
                names=names,
                logical_bytes=int(size),
                created_ns=created_at,
                last_used_ns=created_at,
                active=any(name in used_images for name in names),
                owned=True,
                protected=any(name in used_images for name in names),
            )
        )
    return tuple(sorted(resources, key=lambda row: row.identity))


def _containers(
    policy: DockerRuntimePolicy, runner: CommandRunner
) -> tuple[tuple[RuntimeResource, ...], set[str]]:
    result = runner(
        (
            policy.command,
            "container",
            "ls",
            "-a",
            "--size",
            "--no-trunc",
            "--format",
            "{{json .}}",
        ),
        policy.timeout_seconds,
    )
    if result.returncode != 0:
        raise ValueError(result.stderr or "Docker container inventory failed")
    resources = []
    used_images = set()
    for raw in json_lines(result.stdout):
        used_images.add(str(raw.get("Image", "")))
        name = str(raw.get("Names", ""))
        if not any(name.startswith(prefix) for prefix in policy.container_prefixes):
            continue
        active = str(raw.get("State", "")).lower() == "running"
        created = timestamp(str(raw.get("CreatedAt", "")))
        resources.append(
            RuntimeResource(
                kind=ResourceKind.CONTAINER,
                identity=str(raw["ID"]),
                names=(name,),
                logical_bytes=parse_size(str(raw.get("Size", "0B"))),
                created_ns=created,
                last_used_ns=created,
                active=active,
                owned=True,
                protected=active,
            )
        )
    return tuple(resources), used_images


def inventory(
    runtime_id: str,
    policy: DockerRuntimePolicy,
    *,
    runner: CommandRunner = execute,
    now_ns: int | None = None,
) -> RuntimeInventory:
    generated = time.time_ns() if now_ns is None else now_ns
    try:
        storage = categories(policy, runner=runner)
        containers, used_images = _containers(policy, runner)
        images = _owned_images(policy, runner, used_images)
        volumes = dockervolumes.inventory(policy, runner=runner)
    except (KeyError, TypeError, ValueError, json.JSONDecodeError) as error:
        return RuntimeInventory(
            runtime_id=runtime_id,
            kind=RuntimeKind.DOCKER,
            available=False,
            generated_ns=generated,
            native_bytes=0,
            owned_bytes=0,
            error=str(error),
        )
    build = next((row for row in storage if row.name == "Build Cache"), None)
    build_resource = (
        (
            RuntimeResource(
                kind=ResourceKind.BUILD_CACHE,
                identity="buildkit",
                names=("BuildKit shared cache",),
                logical_bytes=build.logical_bytes,
                created_ns=0,
                last_used_ns=0,
                active=False,
                owned=True,
                protected=False,
            ),
        )
        if policy.build_cache_owned and build is not None
        else ()
    )
    resources = (*images, *containers, *volumes, *build_resource)
    return RuntimeInventory(
        runtime_id=runtime_id,
        kind=RuntimeKind.DOCKER,
        available=True,
        generated_ns=generated,
        native_bytes=sum(row.logical_bytes for row in storage),
        owned_bytes=sum(row.logical_bytes for row in resources),
        categories=storage,
        resources=resources,
    )
