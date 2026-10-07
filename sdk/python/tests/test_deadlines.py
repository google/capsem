"""HTTP readiness and command deadlines retain cancellation and mutation identity."""

from __future__ import annotations

import asyncio
import json
from typing import Any

import pytest
from capsem import VM, Hypervisor, models
from capsem.execution import (
    CREATE_READY_SECS,
    GATEWAY_REQUEST_BUDGET_SECS,
    command_deadline,
)

from .facade_gateway import gateway


def test_exec_outlives_the_default_deadline_without_replaying_it() -> None:
    async def run() -> None:
        async with gateway() as (url, state), VM(url, "token", id="vm-0", timeout=0.05) as vm:
            state.delays["/vms/vm-0/exec"] = 0.3
            await vm.exec("slow build", timeout_secs=600)
            state.delays["/run"] = 0.3
            async with Hypervisor(url, "token", timeout=0.05) as hv:
                await hv.run("slow build")
            assert [path for _, path, _ in state.requests] == ["/vms/vm-0/exec", "/run"]
            assert json.loads(state.requests[-1][2]) == {
                "command": "slow build", "timeout_secs": None, "cpus": None, "ram_mb": None, "env": None,
            }
    asyncio.run(run())


def test_create_outlives_the_default_deadline_like_exec(monkeypatch: pytest.MonkeyPatch) -> None:
    async def run() -> None:
        async with gateway() as (url, state), Hypervisor(url, "token", timeout=0.05) as hv:
            state.delays["/vms/create"] = 0.3
            seen: list[float | None] = []
            request = hv._transport.request

            async def spy(*args: Any, **kwargs: Any) -> bytes:
                seen.append(kwargs.get("timeout"))
                return await request(*args, **kwargs)

            monkeypatch.setattr(hv._transport, "request", spy)
            assert (await hv.create(image="docker://busybox:latest")).id == "created-id"
            # 0.7 creates directly against its one runtime: there is no
            # preliminary profile lookup before the create request.
            assert seen == [CREATE_READY_SECS + GATEWAY_REQUEST_BUDGET_SECS]
            assert [request[1] for request in state.requests] == ["/vms/create"]
    asyncio.run(run())


def test_ordinary_calls_keep_the_default_deadline() -> None:
    async def run() -> None:
        async with gateway() as (url, state), VM(url, "token", id="vm-0", timeout=0.05) as vm:
            state.delays["/vms/vm-0/info"] = 0.3
            with pytest.raises(TimeoutError):
                await vm.info()
            assert len(state.requests) == 1
    asyncio.run(run())


@pytest.mark.parametrize("operation", ["start", "resume"])
@pytest.mark.parametrize("fallback", [0.05, 500])
def test_restore_covers_workload_readiness_without_replay(
    operation: str, fallback: float, monkeypatch: pytest.MonkeyPatch,
) -> None:
    async def run() -> None:
        async with gateway() as (url, state), VM(url, "token", id="vm-0", timeout=fallback) as vm:
            path = f"/vms/vm-0/{operation}"
            state.delays[path] = 0.3
            seen: list[float | None] = []
            request = vm._transport.request

            async def spy(*args: Any, **kwargs: Any) -> bytes:
                timeout = kwargs.get("timeout")
                seen.append(fallback if timeout is None else timeout)
                return await request(*args, **kwargs)

            monkeypatch.setattr(vm._transport, "request", spy)
            assert isinstance(await getattr(vm, operation)(), models.ProvisionResponse)
            assert seen == [max(fallback, CREATE_READY_SECS + GATEWAY_REQUEST_BUDGET_SECS)]
            assert state.requests == [("POST", path, b"")]
    asyncio.run(run())


@pytest.mark.parametrize("operation", ["start", "resume"])
def test_restore_cancellation_does_not_replay_or_close_the_parent(operation: str) -> None:
    async def run() -> None:
        async with gateway() as (url, state), VM(url, "token", id="vm-0", timeout=0.05) as vm:
            path = f"/vms/vm-0/{operation}"
            state.delays[path] = 0.3
            pending = asyncio.create_task(getattr(vm, operation)())
            while not state.requests:
                await asyncio.sleep(0)
            pending.cancel()
            with pytest.raises(asyncio.CancelledError):
                await pending
            assert isinstance(await vm.info(), models.SandboxInfo)
            assert state.requests == [("POST", path, b""), ("GET", "/vms/vm-0/info", b"")]
    asyncio.run(run())


@pytest.mark.parametrize(
    ("default", "timeout_secs", "expected"),
    [(30, None, 3600 + 120), (30, 600, 600 + 120), (5000, 10, 5000)],
)
def test_command_deadline_covers_the_service_timeout_and_gateway_budget(
    default: float, timeout_secs: int | None, expected: float,
) -> None:
    assert command_deadline(default, timeout_secs) == expected


