"""SDK controller gateway resolution, VM lifecycle, and session binding unit tests."""

from __future__ import annotations

from types import SimpleNamespace
from typing import Any, cast

import capsem
import inspect_capsem._controller as ctrl_mod
import inspect_capsem.sandbox as sb_mod
import pytest
from capsem import CreateTimeoutError, HttpError, VmNotFoundError, models
from inspect_capsem import CapsemSandboxEnvironment
from inspect_capsem._controller import SdkCapsemController

from .helpers import Scripted, exec_response, sandbox_info


async def test_sdk_controller_builds_default_hypervisor(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("CAPSEM_GATEWAY_URL", "http://127.0.0.1:1")
    monkeypatch.setenv("CAPSEM_GATEWAY_TOKEN", "tok")
    connect_calls: list[dict[str, Any]] = []
    real_connect = capsem.Hypervisor.connect

    def spy_connect(url: str | None = None, token: str | None = None, **kw: Any) -> Any:
        connect_calls.append({"url": url, "token": token, **kw})
        return real_connect(url, token, **kw)

    monkeypatch.setattr(capsem.Hypervisor, "connect", spy_connect)
    ctrl = SdkCapsemController()
    try:
        assert isinstance(ctrl._hypervisor, capsem.Hypervisor)
        assert connect_calls == [
            {"url": None, "token": None, "timeout": ctrl_mod._SDK_CALL_TIMEOUT_SECS}
        ]
    finally:
        await ctrl.close()


async def test_sdk_controller_ephemeral_create_and_labels(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    created: list[dict[str, Any]] = []
    monkeypatch.delenv("CAPSEM_VM_PREFIX", raising=False)

    async def fake_create(self: Any, **kwargs: Any) -> Any:
        del self
        created.append(kwargs)
        return SimpleNamespace(id=f"eph-{len(created)}", name=kwargs.get("name") or None)

    async def broken_close(self: Any) -> None:
        del self
        raise RuntimeError("close failure is ignored")

    monkeypatch.setattr(capsem.Hypervisor, "create", fake_create)
    monkeypatch.setattr(capsem.Hypervisor, "close", broken_close)
    ctrl = SdkCapsemController(hypervisor=capsem.Hypervisor("http://127.0.0.1:1", "t"))
    try:
        vid = await ctrl.start_vm(cpu_count=1, ram_gb=1)
        assert vid == "eph-1" and created[-1] == {
            "cpus": 1,
            "memory": 1,
            "labels": {"managed-by": "inspect-capsem"},
        }
        vid_env = await ctrl.start_vm(
            cpu_count=2,
            ram_gb=4,
            env={"FOO": "bar"},
            labels={"inspect-capsem-task": "task-1"},
        )
        assert vid_env == "eph-2" and "name" not in created[-1]
        assert created[-1]["labels"] == {
            "managed-by": "inspect-capsem",
            "inspect-capsem-task": "task-1",
        }
        assert created[-1]["env"] == {"FOO": "bar"}
    finally:
        await ctrl.close()


async def test_sdk_create_cleanup_on_504(monkeypatch: pytest.MonkeyPatch) -> None:
    """When create raises CreateTimeoutError with vm_id, _cleanup_failed_create stops it."""
    deleted_ids: list[str] = []
    mode = {"kind": "504"}

    async def create_vm(self: Any, **kwargs: Any) -> Any:
        del self, kwargs
        if mode["kind"] == "504":
            raise CreateTimeoutError("orphan-504 timed out", vm_id="orphan-504", status=504)
        if mode["kind"] == "504_no_vm":
            raise CreateTimeoutError("gateway timeout", vm_id=None, status=504)
        raise RuntimeError("VM creation failed")

    async def delete_vm(self: Any) -> Any:
        assert self.id is not None
        deleted_ids.append(self.id)
        return models.VmActionResponse(success=True)

    monkeypatch.setattr(capsem.Hypervisor, "create", create_vm)
    monkeypatch.setattr(capsem.VM, "delete", delete_vm)
    ctrl = SdkCapsemController(hypervisor=capsem.Hypervisor("http://127.0.0.1:1", "t"))
    try:
        with pytest.raises(CreateTimeoutError, match="orphan-504") as exc_info:
            await ctrl.start_vm(cpu_count=1, ram_gb=1)
        assert exc_info.value.vm_id == "orphan-504" and deleted_ids == ["orphan-504"]
        deleted_ids.clear()
        mode["kind"] = "504_no_vm"
        with pytest.raises(CreateTimeoutError, match="gateway timeout"):
            await ctrl.start_vm(cpu_count=1, ram_gb=1)
        assert deleted_ids == []
        mode["kind"] = "non_504"
        with pytest.raises(RuntimeError, match="VM creation failed"):
            await ctrl.start_vm(cpu_count=1, ram_gb=1)
        assert deleted_ids == []
    finally:
        await ctrl.close()


async def test_sdk_controller_prefix_labels(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """SdkCapsemController isolates CAPSEM_VM_PREFIX via labels."""
    last_create: dict[str, Any] = {}

    async def _create(**kwargs: Any) -> Any:
        last_create.update(kwargs)
        return SimpleNamespace(id="vm-ephemeral-1", name=kwargs.get("name") or None)

    async def _close() -> None:
        return None

    def _lbl(prefix: str = "") -> dict[str, str]:
        base = {"managed-by": "inspect-capsem"}
        if prefix:
            base["inspect-capsem-prefix"] = prefix
        return base

    ctrl = SdkCapsemController(hypervisor=cast(Any, SimpleNamespace(create=_create, close=_close)))
    try:
        monkeypatch.setenv("CAPSEM_VM_PREFIX", "bench-a")
        assert ctrl_mod._managed_vm_prefix_slug() == "bench-a"
        assert ctrl_mod._managed_vm_labels(task_name="t1") == {
            **_lbl("bench-a"),
            "inspect-capsem-task": "t1",
        }
        vid = await ctrl.start_vm(cpu_count=2, ram_gb=4)
        assert vid == "vm-ephemeral-1" and last_create["labels"] == _lbl("bench-a")
        ctrl._registry_ca_pem = "CA-CTOR"
        await ctrl.start_vm(cpu_count=1, ram_gb=1, image="b:1")
        assert last_create["registry"].ca_pem == "CA-CTOR"
        await ctrl.start_vm(cpu_count=1, ram_gb=1, image="b:1", registry_ca_pem="CA-ARG")
        assert last_create["registry"].ca_pem == "CA-ARG"
        ctrl._registry_ca_pem = None

        for raw, slug in (
            ("inspect-capsem-run_2/x-", "run-2-x"),
            ("inspect-capsem----", ""),
            ("   ", ""),
        ):
            monkeypatch.setenv("CAPSEM_VM_PREFIX", raw)
            assert ctrl_mod._managed_vm_prefix_slug() == slug

        monkeypatch.setenv("CAPSEM_VM_PREFIX", "run")
        scripted = Scripted()
        stopped = models.VmLifecycleState.STOPPED
        u1 = "550e8400-e29b-41d4-a716-446655440001"
        u2 = "550e8400-e29b-41d4-a716-446655440002"
        scripted.vms = [
            sandbox_info(u1, "vm-1", status=stopped, labels=_lbl("run")),
            sandbox_info(u2, "vm-2", status=stopped, labels=_lbl("run-2")),
            sandbox_info("vm-default-stopped", "vm-3", status=stopped, labels=_lbl()),
            sandbox_info("vm-user-running", "user-dev-vm", labels={}),
            sandbox_info(
                "vm-named-persisted",
                "saved-vm",
                status=stopped,
                persistent=True,
                labels=_lbl("run"),
            ),
        ]
        assert await sb_mod.sweep_leftover_vms(scripted) == [u1]

        scripted.stopped.clear()
        scripted.vms = [
            sandbox_info(u1, "vm-1", labels=_lbl("run")),
            sandbox_info(u2, "vm-2", labels=_lbl("run-2")),
            sandbox_info("vm-user-live", "user-dev-vm", labels={}),
            sandbox_info("vm-named-live", "saved-vm", persistent=True, labels=_lbl("run")),
        ]
        monkeypatch.setattr(sb_mod, "SdkCapsemController", lambda: scripted)
        CapsemSandboxEnvironment._active_environments.clear()
        await CapsemSandboxEnvironment.cli_cleanup(None)
        assert scripted.stopped == [u1]

        monkeypatch.delenv("CAPSEM_VM_PREFIX", raising=False)
        scripted.stopped.clear()
        scripted.vms = [
            sandbox_info(u1, "vm-1", status=stopped, labels=_lbl()),
            sandbox_info(u2, "vm-2", status=stopped, labels=_lbl("run2")),
        ]
        assert await sb_mod.sweep_leftover_vms(scripted) == [u1]
    finally:
        await ctrl.close()


async def test_sdk_controller_sessions_stop_list_and_404(monkeypatch: pytest.MonkeyPatch) -> None:
    deleted_ids: list[str] = []
    written_files: dict[tuple[str | None, str], bytes] = {}
    hv = capsem.Hypervisor("http://127.0.0.1:1", "t")
    files_cls = type(hv.vm(id="probe").files)
    managed = {"managed-by": "inspect-capsem"}
    b64 = models.ExecOutputEncoding.BASE64

    async def fake_list(self: Any) -> Any:
        del self
        return models.ListResponse(
            sandboxes=[
                sandbox_info("r1", "r1", status=models.VmLifecycleState.RUNNING),
                sandbox_info("x1", None, status=models.VmLifecycleState.STOPPED, labels=managed),
                sandbox_info("x2", None, status=models.VmLifecycleState.DEFUNCT, labels=managed),
            ]
        )

    async def fake_exec(
        self: Any, command: str, *, timeout_secs: int | None = None, target: Any = None
    ) -> Any:
        del timeout_secs
        assert target == models.ExecTarget.VM
        if self.id == "known":
            return exec_response(4, f"bash {command}", "ZQ==", truncated=True, stderr_encoding=b64)
        return exec_response(0, stdout="foreign-ok\n")

    async def fake_delete(self: Any) -> Any:
        assert self.id is not None
        if self.id == "vm-gone":
            raise VmNotFoundError("VM not found", vm_id="vm-gone", status=404)
        if self.id == "vm-raw-404":
            raise HttpError(404, "VM not found")
        if self.id == "vm-boom":
            raise HttpError(500, "internal gateway error")
        deleted_ids.append(self.id)
        return models.VmActionResponse(success=True)

    async def fake_write_file(self: Any, path: str, content: bytes | str) -> None:
        raw = content.encode("utf-8") if isinstance(content, str) else bytes(content)
        written_files[(self._vm.id, path)] = raw

    async def fake_read_bytes(self: Any, path: str) -> bytes:
        return written_files[(self._vm.id, path)]

    monkeypatch.setattr(capsem.Hypervisor, "list", fake_list)
    monkeypatch.setattr(capsem.VM, "exec", fake_exec)
    monkeypatch.setattr(capsem.VM, "delete", fake_delete)
    monkeypatch.setattr(files_cls, "write", fake_write_file)
    monkeypatch.setattr(files_cls, "read", fake_read_bytes)

    ctrl = SdkCapsemController(hypervisor=hv)
    try:
        ctrl._sessions["known"] = hv.vm(id="known")
        res = await ctrl.exec_in_vm("known", "echo x", timeout=5)
        assert (res.exit_code, res.stdout, res.stderr, res.truncated) == (
            4,
            "bash echo x",
            "e",
            True,
        )
        assert [vm.id for vm in await ctrl.list_vms()] == ["r1", "x1", "x2"]
        assert await sb_mod.sweep_leftover_vms(ctrl) == ["x1", "x2"]
        assert deleted_ids == ["x1", "x2"]
        assert "foreign-vm" not in ctrl._sessions
        res_f = await ctrl.exec_in_vm("foreign-vm", "uname -a", timeout=10)
        assert res_f.exit_code == 0 and res_f.stdout == "foreign-ok\n"
        assert isinstance(ctrl._sessions["foreign-vm"], capsem.VM)
        await ctrl.upload_to_vm("foreign-vm", "/root/hello.txt", b"world")
        assert await ctrl.download_from_vm("foreign-vm", "/root/hello.txt") == b"world"
        assert await sb_mod.sweep_process_owned_vms(controller=ctrl) == []
        assert "foreign-vm" not in deleted_ids
        for vid in ("known", "vm-gone", "vm-raw-404"):
            await ctrl.stop_vm(vid)
        with pytest.raises(HttpError, match="internal gateway error"):
            await ctrl.stop_vm("vm-boom")
    finally:
        await ctrl.close()
