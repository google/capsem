"""SDK controller gateway resolution, ManagedSession lifecycle, and session binding unit tests."""

from __future__ import annotations

import http.server
import json
import threading
from types import SimpleNamespace
from typing import Any

import capsem
import capsem_integrations._controller as ctrl_mod
import pytest
from capsem import CreateTimeoutError, HttpError, VmNotFoundError, models
from capsem.execution import decode_exec_output
from capsem.models import ExecOutput, ExecOutputEncoding
from capsem_integrations._controller import SdkCapsemController, truncate_label_value

from .helpers import exec_response


@pytest.mark.parametrize("raw", ["é" * 200, "🙂" * 100, "ascii" * 100])
def test_truncate_label_value_bounds_utf8_bytes_without_splitting_unicode(raw: str) -> None:
    truncated = truncate_label_value(raw)
    assert len(truncated.encode("utf-8")) <= 255
    assert raw.startswith(truncated)
    assert len((truncated + raw[len(truncated)]).encode("utf-8")) > 255
    assert truncate_label_value("short") == "short"


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


async def test_sdk_controller_managed_create_stop_and_close(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    created: list[dict[str, Any]] = []
    closed_managed: list[str] = []

    class FakeManaged:
        def __init__(self, vm_id: str, *, fail_close: bool = False) -> None:
            self.vm_id = vm_id
            self.fail_close = fail_close

        async def __aenter__(self) -> Any:
            return SimpleNamespace(id=self.vm_id)

        async def close(self) -> None:
            if self.fail_close:
                self.fail_close = False
                raise RuntimeError("transient close error")
            closed_managed.append(self.vm_id)

    def fake_managed(self: Any, **kwargs: Any) -> Any:
        del self
        created.append(kwargs)
        vid = f"eph-{len(created)}"
        return FakeManaged(vid, fail_close=(vid == "eph-2"))

    async def broken_close(self: Any) -> None:
        del self
        raise RuntimeError("close failure is ignored")

    monkeypatch.setattr(capsem.Hypervisor, "managed", fake_managed)
    monkeypatch.setattr(capsem.Hypervisor, "close", broken_close)
    ctrl = SdkCapsemController(hypervisor=capsem.Hypervisor("http://127.0.0.1:1", "t"))
    try:
        vid = await ctrl.start_vm(cpu_count=1, ram_gb=1)
        assert vid == "eph-1" and created[-1] == {"cpus": 1, "memory": 1}
        assert vid in ctrl._managed_sessions and vid in ctrl._sessions

        await ctrl.stop_vm(vid)
        assert closed_managed == ["eph-1"]
        assert vid not in ctrl._managed_sessions and vid not in ctrl._sessions

        vid_env = await ctrl.start_vm(
            cpu_count=2,
            ram_gb=4,
            env={"FOO": "bar"},
            labels={"suite": "eval"},
        )
        assert vid_env == "eph-2"
        assert created[-1] == {
            "cpus": 2,
            "memory": 4,
            "env": {"FOO": "bar"},
            "labels": {"suite": "eval"},
        }
        with pytest.raises(RuntimeError, match="transient close error"):
            await ctrl.stop_vm(vid_env)
        assert vid_env in ctrl._managed_sessions
        await ctrl.stop_vm(vid_env)
        assert closed_managed == ["eph-1", "eph-2"]
        assert vid_env not in ctrl._managed_sessions

        vid_close = await ctrl.start_vm(cpu_count=1, ram_gb=1)
        assert vid_close == "eph-3" and vid_close in ctrl._managed_sessions
        ctrl._registry_ca_pem = "CA-CTOR"
        await ctrl.start_vm(cpu_count=1, ram_gb=1, image="b:1", command=("sleep", "1"))
        assert created[-1]["image"] == "docker://b:1"
        assert created[-1]["command"] == ["sleep", "1"]
        assert created[-1]["registry"].ca_pem == "CA-CTOR"
        await ctrl.start_vm(cpu_count=1, ram_gb=1, image="b:1", registry_ca_pem="CA-ARG")
        assert created[-1]["registry"].ca_pem == "CA-ARG"
        ctrl._registry_ca_pem = None
    finally:
        await ctrl.close()
    assert closed_managed == ["eph-1", "eph-2", "eph-3", "eph-4", "eph-5"]
    assert ctrl._managed_sessions == {}


async def test_sdk_controller_managed_enter_failure_propagates(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    mode = {"kind": "504"}

    class FailingManaged:
        async def __aenter__(self) -> Any:
            if mode["kind"] == "504":
                raise CreateTimeoutError("orphan-504 timed out", vm_id="orphan-504", status=504)
            raise RuntimeError("VM creation failed")

        async def close(self) -> None:
            pass

    monkeypatch.setattr(capsem.Hypervisor, "managed", lambda _self, **_kw: FailingManaged())
    ctrl = SdkCapsemController(hypervisor=capsem.Hypervisor("http://127.0.0.1:1", "t"))
    try:
        with pytest.raises(CreateTimeoutError, match="orphan-504"):
            await ctrl.start_vm(cpu_count=1, ram_gb=1)
        assert ctrl._managed_sessions == {} and ctrl._sessions == {}
        mode["kind"] = "runtime"
        with pytest.raises(RuntimeError, match="VM creation failed"):
            await ctrl.start_vm(cpu_count=1, ram_gb=1)
        assert ctrl._managed_sessions == {} and ctrl._sessions == {}
    finally:
        await ctrl.close()


async def test_sdk_controller_sessions_stop_and_404(monkeypatch: pytest.MonkeyPatch) -> None:
    deleted_ids: list[str] = []
    written_files: dict[tuple[str | None, str], bytes] = {}
    hv = capsem.Hypervisor("http://127.0.0.1:1", "t")
    files_cls = type(hv.vm(id="probe").files)
    b64 = models.ExecOutputEncoding.BASE64

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
        assert "foreign-vm" not in ctrl._sessions
        assert ctrl.get_vm_session("known").id == "known"
        res_f = await ctrl.exec_in_vm("foreign-vm", "uname -a", timeout=10)
        assert res_f.exit_code == 0 and res_f.stdout == "foreign-ok\n"
        assert ctrl.get_vm_session("attached-vm").id == "attached-vm"
        assert isinstance(ctrl._sessions["foreign-vm"], capsem.VM)
        await ctrl.upload_to_vm("foreign-vm", "/root/hello.txt", b"world")
        assert await ctrl.download_from_vm("foreign-vm", "/root/hello.txt") == b"world"
        for vid in ("known", "vm-gone", "vm-raw-404"):
            await ctrl.stop_vm(vid)
        assert deleted_ids == ["known"]
        with pytest.raises(HttpError, match="internal gateway error"):
            await ctrl.stop_vm("vm-boom")
    finally:
        await ctrl.close()


async def test_sdk_controller_with_async_capsem_hypervisor_and_persistent_http() -> None:
    utf8 = ExecOutput(data="hé\n", encoding=ExecOutputEncoding.UTF8)
    assert decode_exec_output(utf8).decode("utf-8", errors="replace") == "hé\n"
    b64 = ExecOutput(data="aGk=", encoding=ExecOutputEncoding.BASE64)
    assert decode_exec_output(b64).decode("utf-8", errors="replace") == "hi"

    seen_targets: list[Any] = []
    req_to_vid: dict[str, str] = {}

    class _Handler(http.server.BaseHTTPRequestHandler):
        def _send_json(self, payload: dict[str, Any]) -> None:
            raw = json.dumps(payload).encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(raw)))
            self.end_headers()
            self.wfile.write(raw)

        def do_POST(self) -> None:
            length = int(self.headers.get("Content-Length", "0"))
            body = json.loads(self.rfile.read(length).decode("utf-8")) if length else {}
            gen, exp = "46a98f98-4683-4b09-b319-e235179d76bb", "2099-01-01T00:00:00+00:00"
            if self.path == "/vms/create":
                rid = body["managed"]["request_id"]
                keys = ("request_id", "generation", "expires_at", "lease_seconds")
                own = {
                    **dict(zip(keys, (rid, gen, exp, 60), strict=True)),
                    "renew_after_seconds": 30,
                }
                vid = "sb-2" if body.get("container") else "sb-1"
                req_to_vid[rid] = vid
                vm_keys = ("id", "name", "status", "persistent", "available_actions", "ownership")
                self._send_json(
                    dict(zip(vm_keys, (vid, vid, "Running", False, [], own), strict=True))
                )
            elif self.path.endswith("/close"):
                rid = self.path.split("/")[3]
                ckeys = ("request_id", "generation", "state", "session_id", "evidence_retained")
                vals = (rid, gen, "closed", req_to_vid.get(rid, "sb-1"), False)
                self._send_json(dict(zip(ckeys, vals, strict=True)))
            elif self.path.endswith("/exec"):
                seen_targets.append(body.get("target"))
                out = {"data": f"ok:{body.get('command', '')}\n", "encoding": "utf8"}
                err = {"data": "", "encoding": "utf8"}
                self._send_json({"exit_code": 0, "stdout": out, "stderr": err, "truncated": False})
            else:
                self._send_json({"success": True})

        def log_message(self, format: str, *args: object) -> None:
            del format, args

    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), _Handler)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    url = f"http://127.0.0.1:{srv.server_address[1]}"
    ctrl = SdkCapsemController(hypervisor=capsem.Hypervisor(url, "tok"))
    try:
        vid = await ctrl.start_vm(cpu_count=2, ram_gb=4)
        assert vid == "sb-1"
        res1 = await ctrl.exec_in_vm(vid, "echo hi", timeout=10)
        assert res1.exit_code == 0 and res1.stdout == "ok:echo hi\n"
        vid_oci = await ctrl.start_vm(cpu_count=2, ram_gb=4, image="alpine:3.19")
        assert vid_oci == "sb-2"
        await ctrl.exec_in_vm(vid_oci, "echo oci", timeout=10)
        assert seen_targets == ["vm", "workload"]
        await ctrl.stop_vm(vid)
        await ctrl.stop_vm(vid_oci)
    finally:
        await ctrl.close()
        srv.shutdown()
        srv.server_close()
