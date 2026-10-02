"""SDK controller unit tests for `inspect_capsem._controller`."""

from __future__ import annotations

import asyncio
import os
import subprocess
import time
from collections.abc import Sequence
from pathlib import Path
from types import SimpleNamespace
from typing import Any

import capsem
import capsem._operations as api
import capsem.models as models
import capsem.vm as vm_mod
import inspect_capsem._controller as ctrl_mod
import inspect_capsem.sandbox as sb
import pytest
from capsem._transport import HttpError
from inspect_capsem import CapsemSandboxEnvironment, CommandResult, SdkCapsemController

from .conftest import (
    Scripted,
    _host_timeout_skips,
    _run_inspect_self_check,
    env_for,
    fail,
    ok,
)


def _sandbox_info(
    vid: str,
    name: str | None = None,
    *,
    status: models.VmLifecycleState = models.VmLifecycleState.RUNNING,
    persistent: bool = False,
) -> models.SandboxInfo:
    return models.SandboxInfo(
        id=vid,
        name=name if name is not None else vid,
        status=status,
        persistent=persistent,
        available_actions=[],
        pid=1,
        profile_id="default",
    )


def _exec_response(
    exit_code: int = 0,
    stdout: str = "",
    stderr: str = "",
    *,
    truncated: bool = False,
    stderr_encoding: models.ExecOutputEncoding = models.ExecOutputEncoding.UTF8,
) -> models.ExecResponse:
    return models.ExecResponse(
        exit_code=exit_code,
        stdout=models.ExecOutput(data=stdout, encoding=models.ExecOutputEncoding.UTF8),
        stderr=models.ExecOutput(data=stderr, encoding=stderr_encoding),
        truncated=truncated,
    )


def _patch_default_profile(monkeypatch: pytest.MonkeyPatch, profile_id: str = "default") -> None:
    async def _default_profile_id(self: Any, runtime: str = "vm") -> str:
        del self, runtime
        return profile_id

    monkeypatch.setattr(capsem.Hypervisor, "default_profile_id", _default_profile_id)


def test_inspect_exec_timeout_reaches_capsem() -> None:
    """A 900 s Inspect exec must give Capsem at least 900 s, not the server default."""
    seen: list[int] = []

    class Recorder(Scripted):
        async def exec_in_vm(
            self, vm_id: str, command: str, *, timeout: int = 120
        ) -> CommandResult:
            seen.append(timeout)
            return await super().exec_in_vm(vm_id, command, timeout=timeout)

    env = env_for(Recorder())
    asyncio.run(env.exec(["make"], timeout=900))
    assert seen[-1] >= 900


def test_sdk_exec_forwards_timeout_secs(monkeypatch: pytest.MonkeyPatch) -> None:
    calls: list[int | None] = []
    _patch_default_profile(monkeypatch)

    async def fake_create_vm(transport: Any, *, body: Any, request_timeout: Any = None) -> Any:
        del transport, request_timeout
        return SimpleNamespace(id="s", name=body.name)

    async def fake_exec_vm(
        transport: Any, *, id: str, body: Any, request_timeout: Any = None
    ) -> Any:
        del transport, id, request_timeout
        calls.append(body.timeout_secs)
        return _exec_response(0)

    monkeypatch.setattr(api, "create_vm", fake_create_vm)
    monkeypatch.setattr(api, "exec_vm", fake_exec_vm)

    async def run() -> None:
        ctrl = SdkCapsemController(hypervisor=capsem.Hypervisor("http://127.0.0.1:1", "t"))
        try:
            vid = await ctrl.start_vm(template="code", cpu_count=1, ram_gb=1)
            await ctrl.exec_in_vm(vid, "true", timeout=910)
            await ctrl.exec_in_vm(vid, "true", timeout=3610)
            assert calls == [910, 3600]
        finally:
            await ctrl.close()

    asyncio.run(run())


@pytest.mark.parametrize(("requested", "guest"), [(3600, 3590), (7200, 3590), (900, 900)])
def test_inspect_exec_timeout_stays_within_service_max(requested: int, guest: int) -> None:
    """The service rejects timeout_secs > 3600; the guest `timeout` fires first."""
    seen: list[tuple[int, str]] = []

    class Recorder(Scripted):
        async def exec_in_vm(
            self, vm_id: str, command: str, *, timeout: int = 120
        ) -> CommandResult:
            seen.append((timeout, command))
            return await super().exec_in_vm(vm_id, command, timeout=timeout)

    asyncio.run(env_for(Recorder()).exec(["make"], timeout=requested))
    timeout, command = next(s for s in seen if "timeout -k" in s[1])
    assert timeout == guest + 10 and timeout <= 3600
    assert f"timeout -k 1s {guest}s " in command


def test_gateway_url_and_token_resolution(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.delenv("CAPSEM_GATEWAY_URL", raising=False)
    monkeypatch.delenv("CAPSEM_GATEWAY_TOKEN", raising=False)
    monkeypatch.delenv("CAPSEM_RUN_DIR", raising=False)
    monkeypatch.delenv("CAPSEM_HOME", raising=False)
    assert ctrl_mod._resolve_default_gateway_url("http://x") == "http://x"
    assert ctrl_mod._resolve_default_gateway_url(None) == "http://127.0.0.1:19222"
    assert ctrl_mod._resolve_default_gateway_token("t") == "t"
    assert ctrl_mod._resolve_default_gateway_token(None) == ""
    run = tmp_path / ".capsem" / "run"
    run.mkdir(parents=True)
    (run / "gateway.port").write_text("  \n")
    assert ctrl_mod._resolve_default_gateway_url(None) == "http://127.0.0.1:19222"
    (run / "gateway.port").write_text("4242\n")
    (run / "gateway.token").write_text("filetok\n")
    assert ctrl_mod._resolve_default_gateway_url(None) == "http://127.0.0.1:4242"
    assert ctrl_mod._resolve_default_gateway_token(None) == "filetok"

    # CAPSEM_HOME/run overrides ~/.capsem/run.
    custom_home = tmp_path / "custom-home"
    custom_home_run = custom_home / "run"
    custom_home_run.mkdir(parents=True)
    (custom_home_run / "gateway.port").write_text("5353\n")
    (custom_home_run / "gateway.token").write_text("hometok\n")
    monkeypatch.setenv("CAPSEM_HOME", str(custom_home))
    assert ctrl_mod._resolve_default_gateway_url(None) == "http://127.0.0.1:5353"
    assert ctrl_mod._resolve_default_gateway_token(None) == "hometok"

    # CAPSEM_RUN_DIR overrides both CAPSEM_HOME/run and ~/.capsem/run.
    custom_run = tmp_path / "custom-run"
    custom_run.mkdir(parents=True)
    (custom_run / "gateway.port").write_text("6464\n")
    (custom_run / "gateway.token").write_text("runtok\n")
    monkeypatch.setenv("CAPSEM_RUN_DIR", str(custom_run))
    assert ctrl_mod._resolve_default_gateway_url(None) == "http://127.0.0.1:6464"
    assert ctrl_mod._resolve_default_gateway_token(None) == "runtok"

    monkeypatch.setenv("CAPSEM_GATEWAY_URL", "http://env")
    monkeypatch.setenv("CAPSEM_GATEWAY_TOKEN", "envtok")
    assert ctrl_mod._resolve_default_gateway_url(None) == "http://env"
    assert ctrl_mod._resolve_default_gateway_token(None) == "envtok"


def test_sdk_controller_builds_default_hypervisor(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("CAPSEM_GATEWAY_URL", "http://127.0.0.1:1")
    monkeypatch.setenv("CAPSEM_GATEWAY_TOKEN", "tok")

    async def run() -> None:
        ctrl = SdkCapsemController()
        try:
            assert type(ctrl._hypervisor).__name__ == "Hypervisor"
            assert ctrl._hypervisor._transport.timeout == ctrl_mod._SDK_CALL_TIMEOUT_SECS
        finally:
            await ctrl.close()

    asyncio.run(run())


def test_sdk_controller_profiles_and_create_kwargs(monkeypatch: pytest.MonkeyPatch) -> None:
    created: list[Any] = []
    p1 = models.ProfileSummary.model_construct(id="p1", name="prof-a")
    p2 = models.ProfileSummary.model_construct(id="p2", name="prof-b")

    async def fake_list_profiles(transport: Any) -> Any:
        del transport
        return models.ProfilesListResponse.model_construct(profiles=[p1, p2])

    async def fake_create_vm(transport: Any, *, body: Any, request_timeout: Any = None) -> Any:
        del transport, request_timeout
        created.append(body)
        if body.profile_id == "p1":
            return SimpleNamespace(id="", name="")
        return SimpleNamespace(id=f"eph-{len(created)}", name=body.name)

    async def broken_close(self: Any) -> None:
        del self
        raise RuntimeError("close failure is ignored")

    monkeypatch.setattr(api, "list_profiles", fake_list_profiles)
    monkeypatch.setattr(api, "create_vm", fake_create_vm)
    monkeypatch.setattr(capsem.Hypervisor, "close", broken_close)
    _patch_default_profile(monkeypatch, "code")

    async def run() -> None:
        ctrl = SdkCapsemController(hypervisor=capsem.Hypervisor("http://127.0.0.1:1", "t"))
        try:
            vid = await ctrl.start_vm(template="prof-b", cpu_count=1, ram_gb=1)
            assert vid == "eph-1" and str(created[-1].name).startswith("inspect-capsem-")
            assert created[-1].profile_id == "p2"
            assert created[-1].persistent is True

            vid_oci = await ctrl.start_vm(
                template="code",
                cpu_count=2,
                ram_gb=4,
                image="python:3.12-slim",
                command=("sleep", "infinity"),
                env={"FOO": "bar"},
            )
            assert vid_oci == "eph-2"
            assert created[-1].container.image == "docker://python:3.12-slim"
            assert created[-1].container.args == ["sleep", "infinity"]
            assert created[-1].container.env == {"FOO": "bar"}

            with pytest.raises(ValueError, match="select a VM by exactly one nonempty name or id"):
                await ctrl.start_vm(template="p1", cpu_count=1, ram_gb=1)
            assert created[-1].profile_id == "p1"
        finally:
            await ctrl.close()

    asyncio.run(run())


def test_sdk_controller_template_lookup_failures(monkeypatch: pytest.MonkeyPatch) -> None:
    async def list_profiles_down(transport: Any) -> Any:
        del transport
        raise RuntimeError("profiles down")

    monkeypatch.setattr(api, "list_profiles", list_profiles_down)

    async def run() -> None:
        ctrl = SdkCapsemController(hypervisor=capsem.Hypervisor("http://127.0.0.1:1", "t"))
        try:
            with pytest.raises(RuntimeError, match="profiles down"):
                await ctrl.start_vm(template="custom", cpu_count=1, ram_gb=1)
            with pytest.raises(RuntimeError, match="profiles down"):
                await ctrl._resolve_profile_for_template("x")
        finally:
            await ctrl.close()

    asyncio.run(run())


def test_sdk_controller_sessions_stop_list_and_exec_variants(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    exec_calls: list[tuple[str, str]] = []
    deleted_ids: list[str] = []
    written_files: dict[tuple[str, str], bytes] = {}

    async def fake_list_vms(transport: Any) -> Any:
        del transport
        return models.ListResponse(
            sandboxes=[
                _sandbox_info("d1", "d1", status=models.VmLifecycleState.RUNNING, persistent=False),
                _sandbox_info("o1", "obj", status=models.VmLifecycleState.STOPPED, persistent=True),
            ]
        )

    async def fake_exec_vm(
        transport: Any, *, id: str, body: Any, request_timeout: Any = None
    ) -> Any:
        del transport, request_timeout
        exec_calls.append((id, body.command))
        if id == "known":
            return _exec_response(
                exit_code=4,
                stdout=f"bash {body.command}",
                stderr="ZQ==",
                truncated=True,
                stderr_encoding=models.ExecOutputEncoding.BASE64,
            )
        return _exec_response(0, stdout="foreign-ok\n")

    async def fake_delete_vm(transport: Any, *, id: str) -> Any:
        del transport
        deleted_ids.append(id)
        return models.VmActionResponse(success=True)

    async def fake_write_file(self: Any, path: str, content: bytes | str) -> None:
        vm_id = await self._vm._resolve()
        written_files[(vm_id, path)] = (
            content.encode("utf-8") if isinstance(content, str) else bytes(content)
        )

    async def fake_read_bytes(self: Any, path: str) -> bytes:
        vm_id = await self._vm._resolve()
        return written_files[(vm_id, path)]

    monkeypatch.setattr(api, "list_vms", fake_list_vms)
    monkeypatch.setattr(api, "exec_vm", fake_exec_vm)
    monkeypatch.setattr(api, "delete_vm", fake_delete_vm)
    monkeypatch.setattr(vm_mod.Files, "write", fake_write_file)
    monkeypatch.setattr(vm_mod.Files, "read", fake_read_bytes)

    async def run() -> None:
        hv = capsem.Hypervisor("http://127.0.0.1:1", "t")
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
            ids = [vm["id"] for vm in await ctrl.list_vms()]
            assert ids == ["d1", "o1"]

            # Foreign VM binding via hv.vm(id=...) on a real capsem.Hypervisor transport:
            assert "foreign-vm" not in ctrl._sessions
            res_f = await ctrl.exec_in_vm("foreign-vm", "uname -a", timeout=10)
            assert res_f.exit_code == 0 and res_f.stdout == "foreign-ok\n"
            assert "foreign-vm" in ctrl._sessions
            assert isinstance(ctrl._sessions["foreign-vm"], capsem.VM)
            await ctrl.upload_to_vm("foreign-vm", "/root/hello.txt", b"world")
            assert await ctrl.download_from_vm("foreign-vm", "/root/hello.txt") == b"world"

            await ctrl.stop_vm("known")
            assert "known" in deleted_ids
            await ctrl.stop_vm("missing")
            assert "missing" in deleted_ids
            await ctrl.stop_vm("foreign-vm")
        finally:
            await ctrl.close()

    asyncio.run(run())


def test_sdk_controller_maps_gateway_errors_and_timeouts(monkeypatch: pytest.MonkeyPatch) -> None:
    _patch_default_profile(monkeypatch)

    async def fake_create_vm(transport: Any, *, body: Any, request_timeout: Any = None) -> Any:
        del transport, request_timeout
        return SimpleNamespace(id="s", name=body.name)

    async def fake_exec_vm(
        transport: Any, *, id: str, body: Any, request_timeout: Any = None
    ) -> Any:
        del transport, id, request_timeout
        if body.command == "slow":
            raise HttpError(500, "IPC command timed out after 1s")
        if "slow504" in body.command:
            raise HttpError(504, "Gateway Timeout")
        if "slow408" in body.command:
            raise HttpError(408, "Request Timeout")
        if "false_positive_500" in body.command:
            raise HttpError(500, "database connection timed out in worker")
        if "hang" in body.command:
            await asyncio.sleep(10)
        raise HttpError(500, "other failure")

    monkeypatch.setattr(api, "create_vm", fake_create_vm)
    monkeypatch.setattr(api, "exec_vm", fake_exec_vm)

    async def run() -> None:
        ctrl = SdkCapsemController(hypervisor=capsem.Hypervisor("http://127.0.0.1:1", "t"))
        try:
            vid = await ctrl.start_vm(template="code", cpu_count=1, ram_gb=1)
            with pytest.raises(TimeoutError, match="timed out"):
                await ctrl.exec_in_vm(vid, "slow", timeout=1)
            with pytest.raises(TimeoutError, match="timed out"):
                await ctrl.exec_in_vm(vid, "slow504", timeout=1)
            with pytest.raises(TimeoutError, match="timed out"):
                await ctrl.exec_in_vm(vid, "slow408", timeout=1)
            with pytest.raises(HttpError, match="database connection timed out"):
                await ctrl.exec_in_vm(vid, "false_positive_500", timeout=1)
            with pytest.raises(HttpError):
                await ctrl.exec_in_vm(vid, "fast", timeout=1)

            monkeypatch.setattr(ctrl_mod, "GATEWAY_REQUEST_BUDGET_SECS", 0)
            with pytest.raises(TimeoutError, match="Capsem SDK call did not finish"):
                await ctrl.exec_in_vm(vid, "hang", timeout=0)

            async def hang_delete(transport: Any, *, id: str) -> Any:
                del transport, id
                await asyncio.sleep(10)

            async def hang_list(transport: Any) -> Any:
                del transport
                await asyncio.sleep(10)

            monkeypatch.setattr(api, "delete_vm", hang_delete)
            monkeypatch.setattr(api, "list_vms", hang_list)
            with pytest.raises(TimeoutError, match="Capsem SDK call did not finish"):
                await ctrl.stop_vm(vid, timeout=0.02)
            with pytest.raises(TimeoutError, match="Capsem SDK call did not finish"):
                await ctrl.list_vms(timeout=0.02)
        finally:
            await ctrl.close()

    asyncio.run(run())


def test_sdk_transfer_reports_assembly_failure(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _patch_default_profile(monkeypatch)

    async def fake_create_vm(transport: Any, *, body: Any, request_timeout: Any = None) -> Any:
        del transport, request_timeout
        return SimpleNamespace(id="s", name=body.name)

    async def fake_write_file(self: Any, path: str, content: bytes | str) -> None:
        del self, path, content

    async def fake_exec_vm(
        transport: Any, *, id: str, body: Any, request_timeout: Any = None
    ) -> Any:
        del transport, id, request_timeout
        code = 1 if "cat " in body.command else 0
        return _exec_response(code, stdout="", stderr="disk full")

    monkeypatch.setattr(api, "create_vm", fake_create_vm)
    monkeypatch.setattr(vm_mod.Files, "write", fake_write_file)
    monkeypatch.setattr(api, "exec_vm", fake_exec_vm)
    monkeypatch.setattr(ctrl_mod, "_XFER_STAGE_DIR", str(tmp_path))

    async def run() -> None:
        ctrl = SdkCapsemController(hypervisor=capsem.Hypervisor("http://127.0.0.1:1", "t"))
        try:
            vid = await ctrl.start_vm(template="code", cpu_count=1, ram_gb=1)
            with pytest.raises(RuntimeError, match="disk full"):
                await ctrl.upload_to_vm(vid, "/dest", b"abc")
        finally:
            await ctrl.close()

    asyncio.run(run())


def test_sdk_create_cleanup_on_504(monkeypatch: pytest.MonkeyPatch) -> None:
    """When create raises HttpError(504) naming a VM id, _cleanup_failed_create stops that VM."""
    deleted_ids: list[str] = []
    mode = {"kind": "504"}
    _patch_default_profile(monkeypatch, "prof")

    async def create_vm(transport: Any, *, body: Any, request_timeout: Any = None) -> Any:
        del transport, body, request_timeout
        if mode["kind"] == "504":
            raise HttpError(504, "container workload for VM orphan-504 did not become ready")
        if mode["kind"] == "504_no_vm":
            raise HttpError(504, "gateway timeout")
        raise RuntimeError("VM name too long (max 504)")

    async def delete_vm(transport: Any, *, id: str) -> Any:
        del transport
        deleted_ids.append(id)
        return models.VmActionResponse(success=True)

    monkeypatch.setattr(api, "create_vm", create_vm)
    monkeypatch.setattr(api, "delete_vm", delete_vm)

    async def run() -> None:
        ctrl = SdkCapsemController(hypervisor=capsem.Hypervisor("http://127.0.0.1:1", "t"))
        try:
            with pytest.raises(HttpError, match="orphan-504"):
                await ctrl.start_vm(template="code", cpu_count=1, ram_gb=1)
            assert deleted_ids == ["orphan-504"]

            deleted_ids.clear()
            mode["kind"] = "504_no_vm"
            with pytest.raises(HttpError, match="gateway timeout"):
                await ctrl.start_vm(template="code", cpu_count=1, ram_gb=1)
            assert deleted_ids == []

            mode["kind"] = "non_504"
            with pytest.raises(RuntimeError, match="VM name too long"):
                await ctrl.start_vm(template="code", cpu_count=1, ram_gb=1)
            assert deleted_ids == []
        finally:
            await ctrl.close()

    asyncio.run(run())


def test_sdk_list_and_stop_leftover_vms(monkeypatch: pytest.MonkeyPatch) -> None:
    """`Hypervisor.list` returns a ListResponse; leftovers are deleted by id via the SDK."""
    deleted_ids: list[str] = []

    async def fake_list_vms(transport: Any) -> Any:
        del transport
        return models.ListResponse(
            sandboxes=[
                _sandbox_info("r1", "r1", status=models.VmLifecycleState.RUNNING, persistent=False),
                _sandbox_info(
                    "x1",
                    "inspect-capsem-9",
                    status=models.VmLifecycleState.STOPPED,
                    persistent=True,
                ),
                _sandbox_info(
                    "x2",
                    "inspect-capsem-dead",
                    status=models.VmLifecycleState.DEFUNCT,
                    persistent=True,
                ),
            ]
        )

    async def fake_delete_vm(transport: Any, *, id: str) -> Any:
        del transport
        deleted_ids.append(id)
        return models.VmActionResponse(success=True)

    monkeypatch.setattr(api, "list_vms", fake_list_vms)
    monkeypatch.setattr(api, "delete_vm", fake_delete_vm)

    async def run() -> None:
        ctrl = SdkCapsemController(hypervisor=capsem.Hypervisor("http://127.0.0.1:1", "t"))
        try:
            assert await ctrl.list_vms() == [
                {"id": "r1", "name": "r1", "status": "Running", "persistent": False},
                {"id": "x1", "name": "inspect-capsem-9", "status": "Stopped", "persistent": True},
                {
                    "id": "x2",
                    "name": "inspect-capsem-dead",
                    "status": "Defunct",
                    "persistent": True,
                },
            ]
            assert await sb.sweep_leftover_vms(ctrl) == ["x1", "x2"]
            assert deleted_ids == ["x1", "x2"]
            vm = ctrl._session_for("x1")
            assert isinstance(vm, capsem.VM) and vm.id == "x1"
        finally:
            await ctrl.close()

    asyncio.run(run())


def test_sdk_direct_stage_dir_transfers(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    stage_dir = tmp_path / "root"
    stage_dir.mkdir()
    monkeypatch.setattr(ctrl_mod, "_XFER_STAGE_DIR", str(stage_dir))
    monkeypatch.setattr(ctrl_mod, "_XFER_PART_BYTES", 8)
    _patch_default_profile(monkeypatch)

    writes: list[tuple[str, bytes]] = []
    execs: list[str] = []

    async def fake_create_vm(transport: Any, *, body: Any, request_timeout: Any = None) -> Any:
        del transport, request_timeout
        return SimpleNamespace(id="vm-direct", name=body.name)

    async def fake_write_file(self: Any, path: str, data: bytes) -> None:
        del self
        writes.append((path, data))

    async def fake_read_file(self: Any, path: str) -> bytes:
        del self
        assert path == "direct/file.bin"
        return b"direct-bytes"

    async def fake_exec_vm(
        transport: Any, *, id: str, body: Any, request_timeout: Any = None
    ) -> Any:
        del transport, id, request_timeout
        execs.append(body.command)
        return _exec_response(0)

    monkeypatch.setattr(api, "create_vm", fake_create_vm)
    monkeypatch.setattr(vm_mod.Files, "write", fake_write_file)
    monkeypatch.setattr(vm_mod.Files, "read", fake_read_file)
    monkeypatch.setattr(api, "exec_vm", fake_exec_vm)

    async def run() -> None:
        sdk_ctrl = SdkCapsemController(hypervisor=capsem.Hypervisor("http://127.0.0.1:1", "t"))
        try:
            vid = await sdk_ctrl.start_vm(template="code", cpu_count=1, ram_gb=1)
            await sdk_ctrl.upload_to_vm(vid, str(stage_dir / "direct" / "file.bin"), b"abc")
            assert writes == [("direct/file.bin", b"abc")]
            assert (
                await sdk_ctrl.download_from_vm(vid, str(stage_dir / "direct" / "file.bin"))
                == b"direct-bytes"
            )
            assert execs == []
        finally:
            await sdk_ctrl.close()

    asyncio.run(run())


def test_sdk_stop_vm_suppresses_404_and_reraises_other_errors(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    async def fake_delete_vm(transport: Any, *, id: str) -> Any:
        del transport
        if id == "vm-gone":
            raise HttpError(404, "VM not found")
        raise HttpError(500, "internal gateway error")

    monkeypatch.setattr(api, "delete_vm", fake_delete_vm)

    async def run() -> None:
        ctrl = SdkCapsemController(hypervisor=capsem.Hypervisor("http://127.0.0.1:1", "t"))
        try:
            await ctrl.stop_vm("vm-gone")
            with pytest.raises(HttpError, match="internal gateway error"):
                await ctrl.stop_vm("vm-boom")
        finally:
            await ctrl.close()

    asyncio.run(run())


def test_staged_transfers_suppress_finally_cleanup_error_and_catch_403_413(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import inspect_capsem._tools as tools_mod

    monkeypatch.setattr(ctrl_mod, "_XFER_STAGE_DIR", "/root")
    monkeypatch.setattr(ctrl_mod, "_XFER_PART_BYTES", 4)

    class FailingCleanupController(Scripted):
        async def exec_in_vm(
            self, vm_id: str, command: str, *, timeout: int = 120
        ) -> CommandResult:
            if command.startswith(("rm -rf ", "rm -f ")):
                raise RuntimeError("VM gone during finally cleanup")
            return await super().exec_in_vm(vm_id, command, timeout=timeout)

    async def _noop_write(_p: str, _b: bytes) -> None:
        return None

    async def _empty_read(_p: str) -> bytes:
        return b""

    async def _fail_part(_p: str, _b: bytes) -> None:
        raise RuntimeError("part write failed")

    async def _read_404(rel: str) -> bytes:
        raise HttpError(404, f"Not found: /root/run-413/{rel}")

    async def _read_plain_413_in_path(rel: str) -> bytes:
        raise RuntimeError(f"permission denied reading /root/413/{rel}")

    async def _read_413(rel: str) -> bytes:
        if rel == "large.bin":
            raise HttpError(413, "Payload Too Large")
        return b"chunk-ok"

    async def _read_403(rel: str) -> bytes:
        if rel == "symlink.bin":
            raise HttpError(403, "Forbidden: symlink")
        return b"sym-ok"

    written_parts: list[tuple[str, bytes]] = []

    async def _write_403(rel: str, data: bytes) -> None:
        if rel == "symlink_dest.bin":
            raise HttpError(403, "Forbidden: symlink parent")
        written_parts.append((rel, data))

    async def run() -> None:
        ctrl_ok_up = Scripted()
        await ctrl_mod._staged_upload(
            ctrl_ok_up, "vm-1", "/opt/out.bin", b"0123456789", _noop_write
        )
        assert len(ctrl_ok_up.commands) == 1
        assert "cat /root/.capsem-xfer-" in ctrl_ok_up.commands[0]
        assert "__ec=$?; rm -rf /root/.capsem-xfer-" in ctrl_ok_up.commands[0]

        # 403 on small direct upload falls back to staged upload
        ctrl_403_up = Scripted()
        await ctrl_mod._staged_upload(
            ctrl_403_up, "vm-1", "/root/symlink_dest.bin", b"hi", _write_403
        )
        assert len(written_parts) == 1
        assert any("cat /root/.capsem-xfer-" in c for c in ctrl_403_up.commands)

        ctrl_up = FailingCleanupController([("cat ", fail(stderr="assembly failed"))])
        with pytest.raises(RuntimeError, match="assembly failed"):
            await ctrl_mod._staged_upload(
                ctrl_up, "vm-1", "/opt/out.bin", b"0123456789", _noop_write
            )

        with pytest.raises(RuntimeError, match="part write failed"):
            await ctrl_mod._staged_upload(
                ctrl_up, "vm-1", "/opt/out.bin", b"0123456789", _fail_part
            )

        ctrl_down = FailingCleanupController([("split ", fail(stderr="split failed"))])
        with pytest.raises(RuntimeError, match="split failed"):
            await ctrl_mod._staged_download(ctrl_down, "vm-1", "/opt/in.bin", _empty_read)

        ctrl_tools_up = FailingCleanupController([("docker exec -i", fail(stderr="write boom"))])
        with pytest.raises(RuntimeError, match="write boom"):
            await tools_mod._upload_via_transfer(ctrl_tools_up, "vm-1", "c1", "/app/x", b"data")

        ctrl_tools_down = FailingCleanupController([("docker exec -u 0", fail(stderr="read boom"))])
        with pytest.raises(PermissionError, match="read boom"):
            await tools_mod._download_via_transfer(ctrl_tools_down, "vm-1", "c1", "/app/x")

        ctrl_split = Scripted([("split -b", ok("part.000000\n"))])
        with pytest.raises(HttpError, match="Not found"):
            await ctrl_mod._staged_download(ctrl_split, "vm-1", "/root/missing.bin", _read_404)
        assert ctrl_split.commands == []

        with pytest.raises(RuntimeError, match="permission denied"):
            await ctrl_mod._staged_download(
                ctrl_split, "vm-1", "/root/missing.bin", _read_plain_413_in_path
            )
        assert ctrl_split.commands == []

        res_bytes = await ctrl_mod._staged_download(
            ctrl_split, "vm-1", "/root/large.bin", _read_413
        )
        assert res_bytes == b"chunk-ok"
        assert any("split -b" in c for c in ctrl_split.commands)

        ctrl_split_403 = Scripted([("split -b", ok("part.000000\n"))])
        res_403 = await ctrl_mod._staged_download(
            ctrl_split_403, "vm-1", "/root/symlink.bin", _read_403
        )
        assert res_403 == b"sym-ok"
        assert any("split -b" in c for c in ctrl_split_403.commands)

    asyncio.run(run())


@pytest.mark.asyncio
async def test_sdk_controller_with_async_capsem_hypervisor_and_vm() -> None:
    from capsem.models import ExecOutput, ExecOutputEncoding

    class FakeAsyncVM:
        id = "vm-async-123"
        name = "sdk-vm-test"
        deleted = False

        async def exec(self, command: str, *, timeout_secs: int | None = None) -> object:
            del timeout_secs
            return type(
                "ExecRes",
                (),
                {
                    "exit_code": 0,
                    "stdout": ExecOutput(data=f"ran:{command}\n", encoding=ExecOutputEncoding.UTF8),
                    "stderr": ExecOutput(data="", encoding=ExecOutputEncoding.UTF8),
                    "duration_ms": 1.0,
                    "truncated": False,
                },
            )()

        async def delete(self) -> None:
            self.deleted = True

    class FakeAsyncHypervisor:
        def __init__(self) -> None:
            self._vm = FakeAsyncVM()

        def vm(self, *, name: str | None = None, id: str | None = None) -> FakeAsyncVM:
            del name, id
            return self._vm

        async def create(
            self, *, name: str = "", cpus: int | None = None, memory: int | None = None
        ) -> FakeAsyncVM:
            del name, cpus, memory
            return self._vm

        async def close(self) -> None:
            return None

    hv = FakeAsyncHypervisor()
    ctrl = SdkCapsemController(hypervisor=hv)
    try:
        vid = await ctrl.start_vm(template="code", cpu_count=2, ram_gb=4)
        assert vid == "vm-async-123"
        res = await ctrl.exec_in_vm(vid, "uname -a", timeout=30)
        assert res.exit_code == 0
        assert res.stdout == "ran:uname -a\n"
        await ctrl.stop_vm(vid)
        assert hv._vm.deleted is True
    finally:
        await ctrl.close()


class _LocalFiles:
    """Files API fake: guest paths relative to the /root stand-in `root`."""

    def __init__(self, root: Path) -> None:
        self.root = root
        self.writes: list[int] = []

    async def write(self, path: str, data: bytes) -> None:
        target = self.root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        self.writes.append(len(data))

    async def read(self, path: str) -> bytes:
        return (self.root / path).read_bytes()


class _LocalSdkVM:
    """SDK VM fake running commands on the host and returning ExecOutput-shaped streams."""

    def __init__(self, root: Path) -> None:
        self.id = "vm-local-sdk"
        self.name = "vm-local-sdk"
        self.files = _LocalFiles(root)
        self.exec_count = 0

    async def exec(self, command: str, *, timeout_secs: int | None = None) -> object:
        self.exec_count += 1
        proc = subprocess.run(
            ["bash", "-c", command], capture_output=True, timeout=timeout_secs, check=False
        )
        from capsem.models import ExecOutput, ExecOutputEncoding

        def _stream(raw: bytes) -> ExecOutput:
            import base64 as b64

            return ExecOutput(data=b64.b64encode(raw).decode(), encoding=ExecOutputEncoding.BASE64)

        return type(
            "ExecRes",
            (),
            {
                "exit_code": proc.returncode,
                "stdout": _stream(proc.stdout),
                "stderr": _stream(proc.stderr),
                "truncated": False,
            },
        )()

    async def delete(self) -> None:
        return None


class _LocalSdkHypervisor:
    def __init__(self, root: Path) -> None:
        self._vm = _LocalSdkVM(root)

    def vm(self, *, name: str | None = None, id: str | None = None) -> _LocalSdkVM:
        del name, id
        return self._vm

    async def create(self, **_: Any) -> object:
        return self._vm

    async def close(self) -> None:
        return None


def test_exec_output_text_decodes_sdk_streams() -> None:
    from capsem.execution import decode_exec_output
    from capsem.models import ExecOutput, ExecOutputEncoding

    assert (
        decode_exec_output(ExecOutput(data="hé\n", encoding=ExecOutputEncoding.UTF8)).decode(
            "utf-8", errors="replace"
        )
        == "hé\n"
    )
    b64 = ExecOutput(data="aGk=", encoding=ExecOutputEncoding.BASE64)
    assert decode_exec_output(b64).decode("utf-8", errors="replace") == "hi"


@pytest.mark.asyncio
async def test_sdk_controller_bounds_every_wait(monkeypatch: pytest.MonkeyPatch) -> None:
    class HangingVM:
        id = "vm-hang"

        async def exec(self, command: str, *, timeout_secs: int | None = None) -> object:
            del command, timeout_secs
            await asyncio.sleep(3600)
            raise AssertionError("unreachable")

        async def delete(self) -> None:
            return None

    class HangingHypervisor:
        def vm(self, *, name: str | None = None, id: str | None = None) -> HangingVM:
            del name, id
            return HangingVM()

        async def create(self, **_: Any) -> object:
            return HangingVM()

        async def close(self) -> None:
            return None

    ctrl = SdkCapsemController(hypervisor=HangingHypervisor())
    try:
        vid = await ctrl.start_vm(template="code", cpu_count=1, ram_gb=1)
        monkeypatch.setattr(ctrl_mod, "GATEWAY_REQUEST_BUDGET_SECS", 0.2)
        t0 = time.monotonic()
        with pytest.raises(TimeoutError):
            await ctrl.exec_in_vm(vid, "true", timeout=0)
        assert time.monotonic() - t0 < 5
    finally:
        await ctrl.close()


@pytest.mark.asyncio
async def test_sdk_controller_file_transfer_self_check(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Writes and reads go through Files API parts, not base64 exec chunks."""
    stage_root = tmp_path / "root"
    stage_root.mkdir()
    monkeypatch.setattr(ctrl_mod, "_XFER_STAGE_DIR", str(stage_root))
    monkeypatch.setattr(ctrl_mod, "_XFER_PART_BYTES", 1024)
    hv = _LocalSdkHypervisor(stage_root)
    ctrl = SdkCapsemController(hypervisor=hv)
    work = tmp_path / "work"
    work.mkdir()
    try:
        vid = await ctrl.start_vm(template="code", cpu_count=1, ram_gb=1)
        data = os.urandom(5000)
        await ctrl.upload_to_vm(vid, str(work / "sub" / "blob.bin"), data)
        assert (work / "sub" / "blob.bin").read_bytes() == data
        assert hv._vm.files.writes == [1024, 1024, 1024, 1024, 904]
        assert await ctrl.download_from_vm(vid, str(work / "sub" / "blob.bin")) == data
        await ctrl.upload_to_vm(vid, str(work / "empty.bin"), b"")
        assert await ctrl.download_from_vm(vid, str(work / "empty.bin")) == b""
        assert list(stage_root.iterdir()) == []

        env = CapsemSandboxEnvironment(
            vm_id=vid, controller=ctrl, working_dir=str(work), execution_mode="vm"
        )
        before = hv._vm.exec_count
        big = os.urandom(200_000)
        await env.write_file("big.bin", big)
        assert await env.read_file("big.bin", text=False) == big
        assert hv._vm.exec_count - before < 12
        results = await _run_inspect_self_check(
            env,
            skip={
                "test_read_and_write_large_file_binary",
                "test_exec_input_large",
                "test_exec_as_user",
            }
            | _host_timeout_skips(),
        )
        failures = {k: v for k, v in results.items() if v is not True}
        assert failures == {}, f"self_check via SDK transfer failed: {failures}"
    finally:
        await ctrl.close()


@pytest.mark.asyncio
async def test_sdk_controller_reuses_persistent_event_loop_with_aiohttp() -> None:
    """SdkCapsemController reuses a persistent event loop across real Transport / aiohttp calls."""
    import http.server
    import threading

    from capsem._transport import MediaType, Method, Transport
    from capsem.models import ExecOutput, ExecOutputEncoding

    class _Handler(http.server.BaseHTTPRequestHandler):
        def do_POST(self) -> None:
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(b'{"ok": true}')

        def log_message(self, format: str, *args: object) -> None:
            del format, args

    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), _Handler)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    url = f"http://127.0.0.1:{srv.server_address[1]}"
    transport = Transport(url, "tok")

    class TransportBackedVM:
        id = "sb-1"
        name = "test"

        async def exec(self, command: str, *, timeout_secs: int | None = None) -> object:
            del timeout_secs
            await transport.request(Method.POST, "/exec", accept=MediaType.JSON)
            return type(
                "ExecRes",
                (),
                {
                    "exit_code": 0,
                    "stdout": ExecOutput(data=f"ok:{command}\n", encoding=ExecOutputEncoding.UTF8),
                    "stderr": ExecOutput(data="", encoding=ExecOutputEncoding.UTF8),
                    "duration_ms": 1.0,
                    "truncated": False,
                },
            )()

        async def delete(self) -> None:
            await transport.request(Method.POST, "/delete", accept=MediaType.JSON)

    class TransportBackedHypervisor:
        def vm(self, *, name: str | None = None, id: str | None = None) -> TransportBackedVM:
            del name, id
            return TransportBackedVM()

        async def create(
            self, *, name: str = "", cpus: int | None = None, memory: int | None = None
        ) -> TransportBackedVM:
            del name, cpus, memory
            await transport.request(Method.POST, "/create", accept=MediaType.JSON)
            return TransportBackedVM()

        async def close(self) -> None:
            await transport.close()

    assert capsem is not None
    ctrl = SdkCapsemController(hypervisor=TransportBackedHypervisor())
    try:
        vid = await ctrl.start_vm(template="code", cpu_count=2, ram_gb=4)
        assert vid == "sb-1"
        res1 = await ctrl.exec_in_vm(vid, "echo hi", timeout=10)
        assert res1.exit_code == 0
        assert res1.stdout == "ok:echo hi\n"
        res2 = await ctrl.exec_in_vm(vid, "echo second", timeout=10)
        assert res2.exit_code == 0
        assert res2.stdout == "ok:echo second\n"
        await ctrl.stop_vm(vid)
    finally:
        await ctrl.close()
        srv.shutdown()
        srv.server_close()


@pytest.mark.asyncio
async def test_sdk_controller_rejects_unknown_template_and_resolves_custom_profile() -> None:
    """SdkCapsemController rejects unknown templates and resolves matching custom profiles."""

    class FakeProfile:
        id = "custom-prof"
        name = "Custom Profile"

    class FakeProfilesMgr:
        async def list(self) -> Sequence[FakeProfile]:
            return [FakeProfile()]

    class FakeHypervisor:
        profiles = FakeProfilesMgr()
        last_profile: Any = None

        async def create(
            self,
            *,
            name: str = "",
            cpus: int | None = None,
            memory: int | None = None,
            profile: Any = None,
        ) -> Any:
            del cpus, memory
            self.last_profile = profile
            return type("VM", (), {"id": f"id-custom{name}", "name": name or None})()

        async def close(self) -> None:
            return None

    hv = FakeHypervisor()
    ctrl = SdkCapsemController(hypervisor=hv)
    try:
        with pytest.raises(NotImplementedError, match="template"):
            await ctrl.start_vm(template="nonexistent-template", cpu_count=2, ram_gb=4)
        with pytest.raises(NotImplementedError, match="harbor"):
            await ctrl.start_vm(template="harbor", cpu_count=2, ram_gb=4)

        vid = await ctrl.start_vm(template="custom-prof", cpu_count=2, ram_gb=4)
        assert vid.startswith("id-custominspect-capsem-")
        assert hv.last_profile is not None
        assert hv.last_profile.id == "custom-prof"
    finally:
        await ctrl.close()
