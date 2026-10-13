"""Tests for structured SDK errors."""

from __future__ import annotations

import asyncio
import json
import pickle
from typing import Any

import pytest
from capsem import (
    CapsemError,
    CapsemTimeoutError,
    CreateTimeoutError,
    ExecTimeoutError,
    HttpError,
    Hypervisor,
    VmNotFoundError,
    models,
)

from .facade_gateway import gateway


def test_vm_not_found_error_on_name_resolution_and_404_routes(monkeypatch: pytest.MonkeyPatch) -> None:
    async def run() -> None:
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            state.names = []
            with pytest.raises(VmNotFoundError) as by_name_err:
                await hv.vm(name="gone").info()
            err = by_name_err.value
            assert isinstance(err, (HttpError, LookupError, CapsemError))
            assert (err.status, err.vm_name, err.vm_id) == (404, "gone", None)

            state.names = ["dup", "dup"]
            with pytest.raises(LookupError, match="found 2") as dup_err:
                await hv.vm(name="dup").info()
            assert not isinstance(dup_err.value, VmNotFoundError)

            vm = hv.vm(id="missing-id")
            with pytest.raises(HttpError) as file_err:
                await vm.files.read("/missing")
            assert not isinstance(file_err.value, VmNotFoundError)

            request = hv._transport.request

            async def non_vm_404(method: Any, path: str, **kwargs: Any) -> bytes:
                if path.endswith(("/logs", "/exec")):
                    raise HttpError(404, json.dumps({"error": "no app log found"}))
                return await request(method, path, **kwargs)

            monkeypatch.setattr(hv._transport, "request", non_vm_404)
            for non_vm_call in (lambda: vm.log(), lambda: vm.exec("true")):
                with pytest.raises(HttpError) as plain_404:
                    await non_vm_call()
                assert not isinstance(plain_404.value, VmNotFoundError)

            async def sandbox_404(method: Any, path: str, **kwargs: Any) -> bytes:
                params = kwargs.get("path_parameters") or {}
                if params.get("id") == "missing-id":
                    body = {"error": "not found", "code": "vm_not_found", "vm_id": "missing-id"}
                    raise HttpError(404, json.dumps(body))
                return await request(method, path, **kwargs)

            monkeypatch.setattr(hv._transport, "request", sandbox_404)
            calls = (
                lambda: vm.info(), lambda: vm.exec("true"), lambda: vm.delete(),
                lambda: vm.fork("child"), lambda: vm.log(), lambda: vm.stats.summary(),
                lambda: vm.files.read("/work/a"), lambda: vm.files.write("/work/a", b"x"), lambda: vm.files.list(),
            )
            for call in calls:
                with pytest.raises(VmNotFoundError) as route_err:
                    await call()
                assert (route_err.value.vm_id, route_err.value.status) == ("missing-id", 404)
                assert route_err.value.code is models.ErrorCode.VM_NOT_FOUND

    asyncio.run(run())


def test_exec_timeout_error_on_http_and_client_deadlines(monkeypatch: pytest.MonkeyPatch) -> None:
    async def run() -> None:
        async with gateway() as (url, _), Hypervisor(url, "token") as hv:
            vm = hv.vm(id="vm-0")

            async def ipc_timeout(method: Any, path: str, **_kwargs: Any) -> bytes:
                body = {"error": "timed out", "code": "exec_timeout", "timeout_secs": 17}
                raise HttpError(504, json.dumps(body))

            monkeypatch.setattr(hv._transport, "request", ipc_timeout)
            with pytest.raises(ExecTimeoutError) as exec_504_coded:
                await vm.exec("sleep 20", timeout_secs=20)
            assert isinstance(exec_504_coded.value, (TimeoutError, CapsemTimeoutError))
            assert (exec_504_coded.value.status, exec_504_coded.value.vm_id) == (504, "vm-0")
            assert (exec_504_coded.value.command, exec_504_coded.value.timeout_secs) == ("sleep 20", 17)

            with pytest.raises(ExecTimeoutError) as run_504_coded:
                await hv.run("sleep 20", timeout_secs=20)
            assert (run_504_coded.value.status, run_504_coded.value.timeout_secs) == (504, 17)

            async def gateway_504(method: Any, path: str, **_kwargs: Any) -> bytes:
                raise HttpError(504, "gateway timeout")

            monkeypatch.setattr(hv._transport, "request", gateway_504)
            with pytest.raises(ExecTimeoutError) as exec_504:
                await vm.exec("sleep 9", timeout_secs=9)
            assert (exec_504.value.status, exec_504.value.timeout_secs) == (504, 9)

            async def non_timeout_500(method: Any, path: str, **_kwargs: Any) -> bytes:
                raise HttpError(500, json.dumps({"error": "IPC command failed without code"}))

            monkeypatch.setattr(hv._transport, "request", non_timeout_500)
            for fn in (lambda: vm.exec("broken"), lambda: hv.run("broken")):
                with pytest.raises(HttpError) as plain_500:
                    await fn()
                assert not isinstance(plain_500.value, ExecTimeoutError)

            async def client_timeout(method: Any, path: str, **_kwargs: Any) -> bytes:
                raise TimeoutError("client deadline exceeded")

            monkeypatch.setattr(hv._transport, "request", client_timeout)
            with pytest.raises(ExecTimeoutError) as exec_client:
                await vm.exec("sleep 11", timeout_secs=11)
            assert not isinstance(exec_client.value, HttpError)
            assert (exec_client.value.status, exec_client.value.timeout_secs) == (None, 11)

            with pytest.raises(ExecTimeoutError) as run_client:
                await hv.run("sleep 12", timeout_secs=12)
            assert (run_client.value.status, run_client.value.timeout_secs) == (None, 12)

    asyncio.run(run())


def test_exception_pickle_roundtrip_and_parsed_error_code() -> None:
    raw_body = json.dumps({"error": "not found", "code": "vm_not_found", "vm_id": "vm-9"})
    http_err = HttpError(404, raw_body)
    assert http_err.code is models.ErrorCode.VM_NOT_FOUND
    assert http_err.response is not None and http_err.response.vm_id == "vm-9"
    for _ in range(2):
        http_err = pickle.loads(pickle.dumps(http_err))
    assert (http_err.status, http_err.body, http_err.code) == (404, raw_body, models.ErrorCode.VM_NOT_FOUND)
    assert str(http_err) == f"HTTP 404: {raw_body}"

    vm_err = VmNotFoundError(raw_body, vm_id="vm-9", vm_name="box", status=404)
    for _ in range(2):
        vm_err = pickle.loads(pickle.dumps(vm_err))
    assert (vm_err.status, vm_err.body, vm_err.vm_id, vm_err.vm_name) == (404, raw_body, "vm-9", "box")
    assert str(vm_err) == f"HTTP 404: {raw_body}"

    create_err = CreateTimeoutError(
        "HTTP 504: timeout", vm_id="vm-1", vm_name="box", deadline_secs=180.0, status=504, body="timeout",
    )
    create_rt = pickle.loads(pickle.dumps(create_err))
    assert isinstance(create_rt, CapsemTimeoutError)
    assert (create_rt.vm_id, create_rt.vm_name, create_rt.deadline_secs, create_rt.status) == (
        "vm-1", "box", 180.0, 504,
    )
    assert str(create_rt) == "HTTP 504: timeout"

    exec_err = ExecTimeoutError(
        "HTTP 504: exec timed out", vm_id="vm-2", command="sleep 30",
        timeout_secs=30, status=504, body="exec timed out",
    )
    exec_rt = pickle.loads(pickle.dumps(exec_err))
    assert isinstance(exec_rt, CapsemTimeoutError)
    assert (exec_rt.vm_id, exec_rt.command, exec_rt.timeout_secs, exec_rt.status) == ("vm-2", "sleep 30", 30, 504)
    assert str(exec_rt) == "HTTP 504: exec timed out"
