"""A live host policy reload bounds native managed owners without a client."""

from __future__ import annotations

import secrets
import time
import uuid

import psutil
import pytest
from helpers.service import ServiceInstance, vm_session_dir

from tests.ironbank.test_managed_network_lifecycle import execute
from tests.ironbank.test_managed_sessions import assert_retired, gateway


def write_policy(service: ServiceInstance, lease_seconds: int, renew_after_seconds: int) -> None:
    (service.home_dir / "settings.toml").write_text(
        f"[managed_sessions]\nlease_seconds = {lease_seconds}\nrenew_after_seconds = {renew_after_seconds}\n"
    )


@pytest.mark.integration
@pytest.mark.parametrize("invalid", [False, True], ids=["narrow-then-relax", "invalid-policy"])
def test_live_managed_policy_reloads_retire_original_native_owner(invalid: bool) -> None:
    service = ServiceInstance()
    request_id, capability = str(uuid.uuid4()), secrets.token_hex(32)
    try:
        service.start()
        client = gateway(service)
        request = {"cpus": 4, "ram_mb": 12288,
                   "managed": {"request_id": request_id, "ownership_token": capability}}
        status, created = client.call_json("POST", "/vms/create", request, timeout=90)
        assert status == 200 and created["ownership"]["lease_seconds"] == 300, created
        session_id = created["id"]
        assert execute(client, session_id, "printf POLICY_NATIVE_READY") == "POLICY_NATIVE_READY"
        directory = vm_session_dir(service.tmp_dir, client, session_id)
        assert service.proc is not None
        native = [process for process in psutil.Process(service.proc.pid).children()
                  if process.name() == "capsem-process"]
        assert len(native) == 1
        native += native[0].children(recursive=True)
        if invalid:
            write_policy(service, 0, 1)
            status, rejected = client.call_json("POST", "/corp/reload", {})
            assert status == 500 and "error" in rejected, rejected
            denied_request = {**request, "managed": {
                "request_id": str(uuid.uuid4()), "ownership_token": secrets.token_hex(32),
            }}
            status, rejected = client.call_json("POST", "/vms/create", denied_request)
            assert status == 500 and rejected["code"] == "store_unavailable", rejected
            status, rejected = client.call_json("POST", f"/vms/managed/{request_id}/claim", {
                "ownership_token": capability,
            })
            assert status == 500 and rejected["code"] == "store_unavailable", rejected
        else:
            write_policy(service, 6, 1)
            status, reloaded = client.call_json("POST", "/corp/reload", {})
            assert status == 200 and reloaded["success"] is True, reloaded
            status, narrowed = client.call_json("POST", f"/vms/managed/{request_id}/claim", {
                "ownership_token": capability,
            })
            assert status == 200 and narrowed["state"] == "active", narrowed
            assert narrowed["ownership"]["generation"] == created["ownership"]["generation"]
            assert narrowed["ownership"]["lease_seconds"] == 6
            assert narrowed["ownership"]["renew_after_seconds"] == 1
            write_policy(service, 300, 60)
            status, reloaded = client.call_json("POST", "/corp/reload", {})
            assert status == 200 and reloaded["success"] is True, reloaded
            status, relaxed = client.call_json("POST", f"/vms/managed/{request_id}/claim", {
                "ownership_token": capability,
            })
            assert status == 200 and relaxed["state"] == "active", relaxed
            assert relaxed["ownership"]["lease_seconds"] == 6
            assert relaxed["ownership"]["renew_after_seconds"] == 5
        deadline = time.monotonic() + 15
        while directory.exists() and time.monotonic() < deadline:
            time.sleep(0.05)
        assert not directory.exists(), "reloaded policy left the original native session alive"
        assert_retired(native)
        status, closed = client.call_json("POST", f"/vms/managed/{request_id}/close", {
            "ownership_token": capability, "outcome": "cancelled",
        }, timeout=30)
        assert status == 200 and closed["state"] == "closed", closed
        assert closed["generation"] == created["ownership"]["generation"]
        assert client.get("/vms/list")["sandboxes"] == []
    finally:
        service.stop()
