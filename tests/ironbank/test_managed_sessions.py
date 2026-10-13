"""Managed close is a native retirement barrier, including service crash recovery."""

from __future__ import annotations

import os
import secrets
import subprocess
import sys
import time
import uuid
from pathlib import Path

import psutil
import pytest
from helpers.bounded import bounded
from helpers.gateway import TcpHttpClient
from helpers.sdk_packages import python_gateway, typescript_gateway
from helpers.service import ServiceInstance, exec_output_text, vm_session_dir
from log_streams import read_log_stream

ROOT = Path(__file__).resolve().parents[2]


def gateway(service: ServiceInstance) -> TcpHttpClient:
    """Use the gateway actually supervised by this service."""
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        try:
            client = TcpHttpClient(
                f"http://127.0.0.1:{int((service.tmp_dir / 'gateway.port').read_text())}",
                (service.tmp_dir / "gateway.token").read_text().strip(),
            )
            if client.get_raw("/vms/list", timeout=2) == 200:
                return client
        except (OSError, ValueError):
            pass
        time.sleep(0.05)
    raise AssertionError(f"service-owned gateway did not start: {service.tmp_dir}")


def assert_retired(processes: list[psutil.Process]) -> None:
    # A zombie has exited in the kernel; a different parent owns its wait.
    alive = []
    for process in processes:
        try:
            if process.is_running() and process.status() != psutil.STATUS_ZOMBIE:
                alive.append(process.pid)
        except psutil.NoSuchProcess:
            pass
    assert not alive, f"close acknowledged before original native children exited: {alive}"


def assert_capability_private(service: ServiceInstance, capability: str, old_log: str) -> None:
    assert capability not in old_log + read_log_stream(service.tmp_dir / "service.log")
    for path in service.tmp_dir.rglob("*.json"):
        assert capability not in path.read_text(), f"raw ownership capability persisted: {path.name}"


@pytest.mark.integration
@pytest.mark.parametrize("crash", [False, True], ids=["close", "service-crash"])
def test_native_managed_close_retires_original_generation(crash: bool) -> None:
    service = ServiceInstance()
    request_id = str(uuid.uuid4())
    capability = secrets.token_hex(32)
    request = {"ram_mb": 12288, "cpus": 4,
               "managed": {"request_id": request_id, "ownership_token": capability}}
    path = f"/vms/managed/{request_id}"
    old_log = ""
    with subprocess.Popen([sys.executable, "-c", "import time; time.sleep(240)"]) as unrelated:
        try:
            service.start()
            client = gateway(service)
            status, created = client.call_json("POST", "/vms/create", request, timeout=90)
            assert status == 200, created
            assert isinstance(created, dict) and created["persistent"] is False
            ownership = created["ownership"]
            assert ownership["request_id"] == request_id
            assert ownership["lease_seconds"] == 300 and ownership["renew_after_seconds"] == 60
            generation = ownership["generation"]
            assert str(uuid.UUID(generation)) == generation
            assert capability not in str(created)
            session_id = created["id"]
            status, executed = client.call_json("POST", f"/vms/{session_id}/exec", {
                "target": "vm", "command": "printf MANAGED_NATIVE_READY", "timeout_secs": 60,
            }, timeout=65)
            assert status == 200 and executed["exit_code"] == 0, executed
            assert exec_output_text(executed) == "MANAGED_NATIVE_READY"
            session_dir = vm_session_dir(service.tmp_dir, client, session_id)
            assert service.proc is not None
            children = psutil.Process(service.proc.pid).children()
            native = [process for process in children if process.name() == "capsem-process"]
            assert len(native) == 1, "managed create must spawn exactly one native VM process"
            native += native[0].children(recursive=True)
            status, denied = client.call_json("POST", path + "/claim", {
                "ownership_token": secrets.token_hex(32),
            })
            assert status == 403 and denied["code"] == "forbidden", denied
            status, claimed = client.call_json("POST", path + "/claim", {
                "ownership_token": capability,
            })
            assert status == 200 and claimed["state"] == "active", claimed
            assert claimed["session"]["id"] == session_id
            assert claimed["ownership"]["generation"] == generation
            status, duplicate = client.call_json("POST", "/vms/create", request)
            assert status == 409 and duplicate["code"] == "duplicate_request", duplicate
            if crash:
                service.proc.kill()
                service.proc.wait(timeout=10)
                service.stop(cleanup=False)
                old_log = read_log_stream(service.tmp_dir / "service.log")
                service.start()
                client = gateway(service)
            close = {"ownership_token": capability, "outcome": "completed"}
            status, closed = client.call_json("POST", path + "/close", close, timeout=90)
            assert status == 200, closed
            assert closed["request_id"] == request_id and closed["generation"] == generation
            assert closed["state"] == "closed" and closed["session_id"] == session_id
            assert isinstance(closed["evidence_retained"], bool)
            assert_retired(native)
            assert not session_dir.exists(), "closed retained the original live session directory"
            assert not any(row["id"] == session_id for row in client.get("/vms/list")["sandboxes"])
            assert unrelated.poll() is None, "owned cleanup killed an unrelated host process"
            assert client.call_json("POST", path + "/close", close) == (200, closed)
            status, duplicate = client.call_json("POST", "/vms/create", request)
            assert status == 409 and duplicate["code"] == "duplicate_request", duplicate
            assert_capability_private(service, capability, old_log)
        finally:
            unrelated.terminate()
            unrelated.wait(timeout=5)
            service.stop()


@pytest.mark.integration
def test_native_close_before_create_cannot_start_a_vm() -> None:
    service = ServiceInstance()
    request_id = str(uuid.uuid4())
    capability = secrets.token_hex(32)
    try:
        service.start()
        client = gateway(service)
        status, closed = client.call_json("POST", f"/vms/managed/{request_id}/close", {
            "ownership_token": capability, "outcome": "cancelled",
        })
        assert status == 200 and closed["state"] == "closed", closed
        assert closed["request_id"] == request_id and "session_id" not in closed
        status, rejected = client.call_json("POST", "/vms/create", {
            "managed": {"request_id": request_id, "ownership_token": capability},
        })
        assert status == 409 and rejected["code"] == "duplicate_request", rejected
        assert client.get("/vms/list")["sandboxes"] == []
        assert_capability_private(service, capability, "")
    finally:
        service.stop()


@pytest.mark.integration
def test_native_managed_lease_expiry_retires_without_client_close() -> None:
    service = ServiceInstance()
    request_id = str(uuid.uuid4())
    capability = secrets.token_hex(32)
    (service.home_dir / "settings.toml").write_text(
        "[managed_sessions]\nlease_seconds = 6\nrenew_after_seconds = 1\n"
    )
    try:
        service.start()
        client = gateway(service)
        status, created = client.call_json("POST", "/vms/create", {
            "cpus": 4, "ram_mb": 12288,
            "managed": {"request_id": request_id, "ownership_token": capability},
        }, timeout=90)
        assert status == 200 and created["ownership"]["lease_seconds"] == 6, created
        session_id = created["id"]
        session_dir = vm_session_dir(service.tmp_dir, client, session_id)
        assert service.proc is not None
        native = [process for process in psutil.Process(service.proc.pid).children()
                  if process.name() == "capsem-process"]
        assert len(native) == 1
        native += native[0].children(recursive=True)
        # No renew or close: public claim observes terminal state without reviving it.
        deadline = time.monotonic() + 15
        while session_dir.exists() and time.monotonic() < deadline:
            time.sleep(0.05)
        assert not session_dir.exists(), "lease expiry left the original session alive"
        assert_retired(native)
        status, claimed = client.call_json("POST", f"/vms/managed/{request_id}/claim", {
            "ownership_token": capability,
        })
        assert status == 200 and claimed["state"] == "closed", claimed
        assert claimed["ownership"]["generation"] == created["ownership"]["generation"]
        assert client.get("/vms/list")["sandboxes"] == []
        assert_capability_private(service, capability, "")
    finally:
        service.stop()


@pytest.mark.integration
@pytest.mark.parametrize("language", ["python", "typescript", "rust"])
def test_sdk_managed_scope_awaits_native_cleanup(language: str) -> None:
    service = ServiceInstance()
    try:
        service.start()
        client = gateway(service)
        environment = {
            **{key: value for key, value in os.environ.items() if key != "VIRTUAL_ENV"},
            "SDK_GATEWAY_URL": client.base_url, "SDK_GATEWAY_TOKEN": client.token,
        }
        if language == "rust":
            result = subprocess.run(
                bounded(["cargo", "run", "--frozen", "--quiet", "-p", "capsem-sdk",
                         "--example", "managed_acceptance"], 120),
                cwd=ROOT, env=environment, capture_output=True, text=True, check=False,
            )
            assert result.returncode == 0, result.stdout + result.stderr
            output = result.stdout
        else:
            run = python_gateway if language == "python" else typescript_gateway
            output = run(
                ROOT, environment,
                probe="managed_acceptance.py" if language == "python" else "managed-acceptance.mjs",
                success_marker="SDK_MANAGED_ACCEPTANCE_OK", timeout_seconds=120,
            )
            assert "SDK_IMAGE_PACKAGE_ACCEPTANCE_OK" in output
        assert "SDK_MANAGED_ACCEPTANCE_OK" in output
        assert client.get("/vms/list")["sandboxes"] == []
        print(output.strip())
    finally:
        service.stop()
