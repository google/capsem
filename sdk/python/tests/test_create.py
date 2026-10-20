"""Structured VM handle recovery on HTTP 504 and client-side create timeouts."""

from __future__ import annotations

import asyncio
import json
from typing import Any

import pytest
from capsem import CreateTimeoutError, HttpError, Hypervisor, models

from .facade_gateway import gateway


def test_create_504_surfaces_vm_id_and_deletes_in_flight_sandbox() -> None:
    async def run() -> None:
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            state.names = []
            body = {"error": "workload for VM vm-504 did not become ready", "code": "create_timeout", "vm_id": "vm-504"}
            state.create_error = (504, json.dumps(body))
            with pytest.raises(CreateTimeoutError) as caught:
                await hv.create(name="box", image="docker://busybox:latest")
            err = caught.value
            assert isinstance(err, TimeoutError)
            assert (err.status, err.vm_id, err.vm_name) == (504, "vm-504", "box")
            assert err.vm is not None and (err.vm.id, err.vm.name) == ("vm-504", "box")
            assert [vm.id for vm in (await hv.list()).sandboxes] == ["vm-504"]
            assert (await err.vm.container.status()).state is models.ContainerState.RUNNING
            await err.vm.delete()
            assert (await hv.list()).sandboxes == []

    asyncio.run(run())


def test_create_504_without_structured_vm_id_late_binds_by_name() -> None:
    async def run() -> None:
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            state.names = ["named"]
            state.create_error = (504, json.dumps({"error": "upstream proxy timed out"}))
            with pytest.raises(CreateTimeoutError) as named_err:
                await hv.create(name="named")
            assert (named_err.value.vm_id, named_err.value.vm_name) == (None, "named")
            assert named_err.value.vm is not None and named_err.value.vm.id is None
            await named_err.value.vm.delete()
            assert named_err.value.vm.id == "vm-0"

            with pytest.raises(CreateTimeoutError) as unnamed_err:
                await hv.create()
            assert (unnamed_err.value.vm_id, unnamed_err.value.vm_name, unnamed_err.value.vm) == (None, None, None)

            state.create_error = (409, json.dumps({"error": 'persistent VM "named" exists'}))
            with pytest.raises(HttpError) as conflict_err:
                await hv.create(name="named")
            assert not isinstance(conflict_err.value, CreateTimeoutError)
            assert conflict_err.value.status == 409

    asyncio.run(run())


def test_create_client_timeout_late_binds_named_vm(monkeypatch: pytest.MonkeyPatch) -> None:
    async def run() -> None:
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            request = hv._transport.request

            async def timeout_on_create(method: Any, path: str, **kwargs: Any) -> bytes:
                if path == "/vms/create":
                    raise TimeoutError("request timed out")
                return await request(method, path, **kwargs)

            monkeypatch.setattr(hv._transport, "request", timeout_on_create)
            state.names = []
            with pytest.raises(CreateTimeoutError) as late_err:
                await hv.create(name="late")
            assert not isinstance(late_err.value, HttpError)
            assert (late_err.value.status, late_err.value.vm_id, late_err.value.vm_name) == (None, None, "late")
            assert late_err.value.vm is not None and late_err.value.vm.id is None
            state.names = ["late"]
            await late_err.value.vm.delete()
            assert late_err.value.vm.id == "vm-0" and (await hv.list()).sandboxes == []

            with pytest.raises(CreateTimeoutError) as unnamed_err:
                await hv.create()
            assert unnamed_err.value.vm is None

    asyncio.run(run())
