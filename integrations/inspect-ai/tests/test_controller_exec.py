"""SDK controller execution, timeout forwarding, gateway error mapping, and HTTP loop tests."""

from __future__ import annotations

import http.server
import json
import os
import subprocess
import threading
from pathlib import Path
from typing import Any, cast

import capsem
import pytest
from capsem import ExecTimeoutError, HttpError, models
from capsem.execution import decode_exec_output
from capsem.models import ExecOutput, ExecOutputEncoding
from inspect_capsem._controller import CommandResult, SdkCapsemController

from .conftest import write_portable_timeout_shim
from .helpers import Scripted, _host_timeout_skips, env_for, exec_response
from .test_sandbox_exec import test_exit_137_sigkill_vs_actual_timeout


async def test_inspect_exec_timeout_reaches_capsem() -> None:
    """A 900 s Inspect exec must give Capsem at least 900 s, not the server default."""
    seen: list[int] = []

    class Recorder(Scripted):
        async def exec_in_vm(
            self, vm_id: str, command: str, *, timeout: int = 120
        ) -> CommandResult:
            seen.append(timeout)
            return await super().exec_in_vm(vm_id, command, timeout=timeout)

    env = env_for(Recorder())
    await env.exec(["make"], timeout=900)
    assert seen[-1] >= 900


async def test_sdk_exec_forwards_timeout_secs(monkeypatch: pytest.MonkeyPatch) -> None:
    calls: list[tuple[int | None, Any]] = []
    hv = capsem.Hypervisor("http://127.0.0.1:1", "t")

    async def fake_create(self: Any, **kwargs: Any) -> Any:
        del kwargs
        return self.vm(id="s")

    async def fake_exec(
        self: Any, command: str, *, timeout_secs: int | None = None, target: Any = None
    ) -> Any:
        del self, command
        calls.append((timeout_secs, target))
        return exec_response(0)

    monkeypatch.setattr(capsem.Hypervisor, "create", fake_create)
    monkeypatch.setattr(capsem.VM, "exec", fake_exec)
    ctrl = SdkCapsemController(hypervisor=hv)
    try:
        vid = await ctrl.start_vm(cpu_count=1, ram_gb=1)
        await ctrl.exec_in_vm(vid, "true", timeout=910)
        await ctrl.exec_in_vm(vid, "true", timeout=3610)
        assert calls == [
            (910, models.ExecTarget.VM),
            (3600, models.ExecTarget.VM),
        ]
    finally:
        await ctrl.close()


@pytest.mark.parametrize(("requested", "guest"), [(3600, 3590), (7200, 3590), (900, 900)])
async def test_inspect_exec_timeout_stays_within_service_max(requested: int, guest: int) -> None:
    """The service rejects timeout_secs > 3600; the guest `timeout` fires first."""
    seen: list[tuple[int, str]] = []

    class Recorder(Scripted):
        async def exec_in_vm(
            self, vm_id: str, command: str, *, timeout: int = 120
        ) -> CommandResult:
            seen.append((timeout, command))
            return await super().exec_in_vm(vm_id, command, timeout=timeout)

    await env_for(Recorder()).exec(["make"], timeout=requested)
    timeout, command = next(s for s in seen if "timeout -k" in s[1])
    assert timeout == guest + 10 and timeout <= 3600
    assert f"timeout -k 1s {guest}s " in command


async def test_sdk_controller_maps_gateway_errors_and_timeouts(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    targets_seen: list[Any] = []

    async def fake_create(self: Any, **kwargs: Any) -> Any:
        del kwargs
        return self.vm(id="s")

    async def fake_exec(
        self: Any, command: str, *, timeout_secs: int | None = None, target: Any = None
    ) -> Any:
        del self, timeout_secs
        targets_seen.append(target)
        if command == "slow":
            raise ExecTimeoutError("IPC command timed out after 1s", command=command, status=504)
        if "slow504" in command:
            raise ExecTimeoutError("Gateway Timeout", command=command, status=504)
        if "slow408" in command:
            raise ExecTimeoutError("Request Timeout", command=command, status=408)
        if "false_positive_500" in command:
            raise HttpError(
                500,
                '{"code": "internal_error", "error": "database connection timed out in worker"}',
            )
        raise HttpError(500, "other failure")

    monkeypatch.setattr(capsem.Hypervisor, "create", fake_create)
    monkeypatch.setattr(capsem.VM, "exec", fake_exec)
    ctrl = SdkCapsemController(hypervisor=capsem.Hypervisor("http://127.0.0.1:1", "t"))
    try:
        vid = await ctrl.start_vm(cpu_count=1, ram_gb=1)
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
        assert all(t == models.ExecTarget.VM for t in targets_seen)
    finally:
        await ctrl.close()


async def test_sdk_controller_with_async_capsem_hypervisor_and_vm() -> None:
    class FakeAsyncVM:
        id = "vm-async-123"
        name = "sdk-vm-test"
        deleted = False

        async def exec(
            self, command: str, *, timeout_secs: int | None = None, target: Any = None
        ) -> object:
            del timeout_secs
            assert target == models.ExecTarget.VM
            return exec_response(0, stdout=f"ran:{command}\n")

        async def delete(self) -> None:
            self.deleted = True

    class FakeAsyncHypervisor:
        def __init__(self) -> None:
            self._vm = FakeAsyncVM()

        def vm(self, *, name: str | None = None, id: str | None = None) -> FakeAsyncVM:
            del name, id
            return self._vm

        async def create(
            self,
            *,
            name: str = "",
            cpus: int | None = None,
            memory: int | None = None,
            labels: Any = None,
        ) -> FakeAsyncVM:
            del name, cpus, memory, labels
            return self._vm

        async def close(self) -> None:
            return None

    hv = FakeAsyncHypervisor()
    ctrl = SdkCapsemController(hypervisor=cast(Any, hv))
    try:
        vid = await ctrl.start_vm(cpu_count=2, ram_gb=4)
        assert vid == "vm-async-123"
        res = await ctrl.exec_in_vm(vid, "uname -a", timeout=30)
        assert res.exit_code == 0 and res.stdout == "ran:uname -a\n"
        await ctrl.stop_vm(vid)
        assert hv._vm.deleted is True
    finally:
        await ctrl.close()


def test_exec_output_text_decodes_sdk_streams() -> None:
    assert (
        decode_exec_output(ExecOutput(data="hé\n", encoding=ExecOutputEncoding.UTF8)).decode(
            "utf-8", errors="replace"
        )
        == "hé\n"
    )
    b64 = ExecOutput(data="aGk=", encoding=ExecOutputEncoding.BASE64)
    assert decode_exec_output(b64).decode("utf-8", errors="replace") == "hi"


async def test_sdk_controller_reuses_persistent_event_loop_with_aiohttp() -> None:
    """SdkCapsemController reuses a persistent event loop across real Hypervisor aiohttp calls."""
    seen_targets: list[Any] = []

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
            if self.path == "/vms/create":
                self._send_json(
                    {"id": "sb-1", "name": "sb-1", "status": "Running", "available_actions": []}
                )
            elif self.path.endswith("/exec"):
                seen_targets.append(body.get("target"))
                out = {"data": f"ok:{body.get('command', '')}\n", "encoding": "utf8"}
                err = {"data": "", "encoding": "utf8"}
                self._send_json({"exit_code": 0, "stdout": out, "stderr": err, "truncated": False})
            else:
                self._send_json({"success": True})

        def do_DELETE(self) -> None:
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
        res2 = await ctrl.exec_in_vm(vid, "echo second", timeout=10)
        assert res2.exit_code == 0 and res2.stdout == "ok:echo second\n"
        assert seen_targets == ["vm", "vm"]
        await ctrl.stop_vm(vid)
    finally:
        await ctrl.close()
        srv.shutdown()
        srv.server_close()


async def test_portable_timeout_shim_and_missing_host_timeout(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Portable timeout shim satisfies macOS host tests when GNU timeout is absent."""
    shim_dir = tmp_path / "shim_bin"
    shim = str(write_portable_timeout_shim(shim_dir))
    assert (
        "GNU coreutils"
        in subprocess.run([shim, "--version"], capture_output=True, text=True).stdout
    )
    ok_res = subprocess.run(
        [shim, "-k", "1s", "5s", "sh", "-c", "printf shim-ok"], capture_output=True, text=True
    )
    assert (ok_res.returncode, ok_res.stdout) == (0, "shim-ok")
    assert (
        subprocess.run([shim, "-k", "0.2s", "0.1s", "sh", "-c", "trap '' TERM; sleep 5"]).returncode
        == 124
    )
    for flag in (["--signal=KILL"], ["-s", "KILL"]):
        assert subprocess.run([shim, *flag, "0.1s", "sleep", "5"]).returncode == 137

    monkeypatch.setenv("PATH", f"{shim_dir}:{os.environ.get('PATH', '/usr/bin:/bin')}")
    assert _host_timeout_skips() == set()
    await test_exit_137_sigkill_vs_actual_timeout(tmp_path)

    def _raise_missing(*_a: Any, **_kw: Any) -> Any:
        raise FileNotFoundError("timeout")

    monkeypatch.setattr(subprocess, "run", _raise_missing)
    assert _host_timeout_skips() == set()
