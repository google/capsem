"""Friendly exec must preserve the gateway's target selection and refusal."""

from __future__ import annotations

import asyncio
import json
from typing import cast

import pytest
from capsem import VM, HttpError, models
from pydantic import ValidationError

from .facade_gateway import gateway


def test_exec_targets_and_omission_use_exact_authenticated_wire_payloads():
    async def run():
        async with gateway() as (url, state), VM(url, "token", id="immutable-id") as vm:
            for target in [models.ExecTarget.VM, models.ExecTarget.WORKLOAD]:
                await vm.exec("printf target", timeout_secs=12, target=target)
                assert state.requests[-1][:2] == ("POST", "/vms/immutable-id/exec")
                assert json.loads(state.requests[-1][2]) == {"command": "printf target", "timeout_secs": 12, "target": target.value}
            await vm.exec("printf default", timeout_secs=12)
            assert json.loads(state.requests[-1][2]) == {"command": "printf default", "timeout_secs": 12}
            await vm.exec("printf default", timeout_secs=12, target=None)
            assert "target" not in json.loads(state.requests[-1][2])
            assert len(state.requests) == 4
    asyncio.run(run())


def test_invalid_target_refuses_before_name_lookup_or_http():
    async def run():
        async with gateway() as (url, state), VM(url, "token", name="named") as vm:
            for target in ["root", "", 1, []]:
                with pytest.raises(ValidationError):
                    await vm.exec("printf no", target=cast(models.ExecTarget, target))
            assert state.requests == []
            assert vm.id is None
    asyncio.run(run())


def test_target_refusal_never_falls_back_or_replays():
    async def run():
        async with gateway() as (url, state), VM(url, "token", id="immutable-id") as vm:
            state.exec_status = 409
            with pytest.raises(HttpError) as captured:
                await vm.exec("printf no", target=models.ExecTarget.WORKLOAD)
            assert captured.value.status == 409
            assert len(state.requests) == 1
            assert json.loads(state.requests[0][2])["target"] == "workload"
    asyncio.run(run())


def test_target_exec_remains_cancellable_without_replay():
    async def run():
        async with gateway() as (url, state), VM(url, "token", id="immutable-id") as vm:
            state.wait_for_exec = True
            pending = asyncio.create_task(vm.exec("sleep 99", target=models.ExecTarget.VM))
            await asyncio.wait_for(state.exec_entered.wait(), 1)
            pending.cancel()
            with pytest.raises(asyncio.CancelledError):
                await pending
            assert len(state.requests) == 1
            assert json.loads(state.requests[0][2])["target"] == "vm"
    asyncio.run(run())
