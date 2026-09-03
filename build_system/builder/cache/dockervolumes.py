"""Inventory persistent Capsem compiler volumes through Docker's native API."""

from __future__ import annotations

import json

from .dockerformat import parse_size, timestamp
from .runtimeexec import CommandRunner
from .runtimemodels import DockerRuntimePolicy, ResourceKind, RuntimeResource


def inventory(
    policy: DockerRuntimePolicy, *, runner: CommandRunner
) -> tuple[RuntimeResource, ...]:
    result = runner(
        (policy.command, "system", "df", "-v", "--format", "{{json .}}"),
        policy.timeout_seconds,
    )
    if result.returncode != 0:
        raise ValueError(result.stderr or "Docker volume inventory failed")
    raw = json.loads(result.stdout)
    volumes = raw.get("Volumes") or []
    resources = []
    for volume in volumes:
        name = str(volume["Name"])
        if not any(name.startswith(prefix) for prefix in policy.volume_prefixes):
            continue
        links = int(str(volume.get("Links", "0")))
        created = timestamp(str(volume.get("CreatedAt", "")))
        resources.append(
            RuntimeResource(
                kind=ResourceKind.VOLUME,
                identity=name,
                names=(name,),
                logical_bytes=parse_size(str(volume.get("Size", "0B"))),
                created_ns=created,
                last_used_ns=created,
                active=links > 0,
                owned=True,
                protected=links > 0,
            )
        )
    return tuple(sorted(resources, key=lambda resource: resource.identity))
