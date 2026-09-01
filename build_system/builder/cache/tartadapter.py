"""Tart VM and OCI-cache inventory without touching runnable state."""

from __future__ import annotations

import json
import os
import time
from datetime import UTC, datetime
from pathlib import Path

from .runtimeexec import CommandRunner, execute
from .runtimemodels import (
    ResourceKind,
    RuntimeInventory,
    RuntimeKind,
    RuntimeResource,
    TartRuntimePolicy,
)


def _timestamp(value: object) -> int:
    if not isinstance(value, str) or not value:
        return 0
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=UTC)
    return int(parsed.timestamp() * 1_000_000_000)


def _allocated_bytes(root: Path) -> int:
    if not root.is_dir():
        return 0
    allocated = 0
    for directory, names, files in os.walk(root, followlinks=False):
        base = Path(directory)
        for name in (*names, *files):
            path = base / name
            if not path.is_symlink():
                allocated += getattr(path.lstat(), "st_blocks", 0) * 512
    return allocated


def inventory(
    runtime_id: str,
    policy: TartRuntimePolicy,
    *,
    runner: CommandRunner = execute,
    now_ns: int | None = None,
) -> RuntimeInventory:
    generated = time.time_ns() if now_ns is None else now_ns
    result = runner((policy.command, "list", "--format", "json"), policy.timeout_seconds)
    if result.returncode != 0:
        return RuntimeInventory(
            runtime_id=runtime_id,
            kind=RuntimeKind.TART,
            available=False,
            generated_ns=generated,
            native_bytes=0,
            owned_bytes=0,
            error=result.stderr or result.stdout or "Tart unavailable",
        )
    try:
        rows = json.loads(result.stdout)
        if not isinstance(rows, list) or any(not isinstance(row, dict) for row in rows):
            raise ValueError("Tart list JSON must be an array of objects")
        resources = []
        for row in rows:
            name = str(row.get("Name", ""))
            source = str(row.get("Source", "")).lower()
            working = source == "local" and any(
                name.startswith(prefix) for prefix in policy.vm_prefixes
            )
            base = any(_is_base(name, reference) for reference in policy.base_images)
            size = row.get("Size", 0)
            if not isinstance(size, (int, float)) or isinstance(size, bool) or size < 0:
                raise ValueError(f"Tart VM {name!r} has invalid Size")
            accessed = _timestamp(row.get("Accessed"))
            running = bool(row.get("Running", False))
            resources.append(
                RuntimeResource(
                    kind=ResourceKind.VM,
                    identity=name,
                    names=(name,),
                    logical_bytes=int(size * 1024**3),
                    created_ns=accessed,
                    last_used_ns=accessed,
                    active=running,
                    owned=working or base,
                    protected=running or base or not working,
                )
            )
    except (TypeError, ValueError, json.JSONDecodeError) as error:
        return RuntimeInventory(
            runtime_id=runtime_id,
            kind=RuntimeKind.TART,
            available=False,
            generated_ns=generated,
            native_bytes=0,
            owned_bytes=0,
            error=str(error),
        )
    values = tuple(sorted(resources, key=lambda row: row.identity))
    native = _allocated_bytes(Path(policy.home).expanduser())
    return RuntimeInventory(
        runtime_id=runtime_id,
        kind=RuntimeKind.TART,
        available=True,
        generated_ns=generated,
        native_bytes=native,
        owned_bytes=sum(row.logical_bytes for row in values if row.owned),
        resources=values,
    )


def _is_base(name: str, reference: str) -> bool:
    repository = reference.split("@", 1)[0]
    return name in {reference, repository} or name.startswith((f"{repository}@", f"{repository}:"))
