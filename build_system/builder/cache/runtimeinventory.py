"""Compose native runtime adapters behind one typed inventory interface."""

from __future__ import annotations

import time

from . import dockeradapter, tartadapter
from .models import CachePolicy
from .runtimeexec import CommandRunner, execute
from .runtimemodels import (
    DockerRuntimePolicy,
    RuntimeInventory,
    RuntimeKind,
    RuntimeSnapshot,
    TartRuntimePolicy,
)


def scan_runtimes(
    policy: CachePolicy,
    *,
    runner: CommandRunner = execute,
    now_ns: int | None = None,
    offline: bool = False,
    runtime_ids: frozenset[str] | None = None,
) -> RuntimeSnapshot:
    generated = time.time_ns() if now_ns is None else now_ns
    inventories: list[RuntimeInventory] = []
    selected = set(policy.runtimes) if runtime_ids is None else set(runtime_ids)
    unknown = sorted(selected - set(policy.runtimes))
    if unknown:
        raise ValueError(f"unknown cache runtimes: {', '.join(unknown)}")
    for runtime_id, runtime in sorted(policy.runtimes.items()):
        if runtime_id not in selected:
            continue
        if offline:
            inventories.append(
                RuntimeInventory(
                    runtime_id=runtime_id,
                    kind=RuntimeKind(runtime.kind),
                    available=False,
                    generated_ns=generated,
                    native_bytes=0,
                    owned_bytes=0,
                    error="offline inventory requested",
                )
            )
        elif isinstance(runtime, DockerRuntimePolicy):
            inventories.append(
                dockeradapter.inventory(runtime_id, runtime, runner=runner, now_ns=generated)
            )
        elif isinstance(runtime, TartRuntimePolicy):
            inventories.append(
                tartadapter.inventory(runtime_id, runtime, runner=runner, now_ns=generated)
            )
        else:  # pragma: no cover - discriminated Pydantic union is exhaustive
            raise AssertionError(f"unsupported runtime policy: {runtime}")
    values = tuple(inventories)
    return RuntimeSnapshot(
        generated_ns=generated,
        native_bytes=sum(item.native_bytes for item in values),
        owned_bytes=sum(item.owned_bytes for item in values),
        runtimes=values,
    )
