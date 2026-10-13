"""Managed cleanup retires native network owners and survives lost create replies."""

from __future__ import annotations

import http.client
import json
import secrets
import time
import uuid
from concurrent.futures import ThreadPoolExecutor

import psutil
import pytest
from helpers.gateway import TcpHttpClient
from helpers.service import ServiceInstance, exec_output_text, vm_session_dir

from tests.ironbank.test_managed_sessions import assert_retired, gateway


def execute(client: TcpHttpClient, session_id: str, command: str) -> str:
    status, result = client.call_json("POST", f"/vms/{session_id}/exec", {
        "target": "vm", "command": command, "timeout_secs": 60,
    }, timeout=65)
    assert status == 200 and result["exit_code"] == 0, result
    return exec_output_text(result)


def switches(service: ServiceInstance) -> list[psutil.Process]:
    assert service.proc is not None
    return [process for process in psutil.Process(service.proc.pid).children()
            if process.name() == "capsem-router" and "--network" in process.cmdline()]


def join(client: TcpHttpClient, network_id: str, vm_id: str) -> dict:
    status, response = client.call_json("PUT", f"/networks/{network_id}/members/{vm_id}")
    assert status == 200, response
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        network = client.get(f"/networks/{network_id}")
        for member in network["members"]:
            if member["vm_id"] == vm_id and member["state"] == "ready":
                return member
        time.sleep(0.05)
    raise AssertionError(f"native private cable never became ready: {network}")


@pytest.mark.integration
def test_managed_close_retires_sole_switch_and_preserves_shared_named_vm() -> None:
    service = ServiceInstance()
    request_id, capability = str(uuid.uuid4()), secrets.token_hex(32)
    try:
        service.start()
        client = gateway(service)
        status, named = client.call_json("POST", "/vms/create", {
            "name": "unrelated-named", "cpus": 4, "ram_mb": 12288,
        }, timeout=90)
        assert status == 200 and named["persistent"], named
        execute(client, named["id"], "printf NAMED_SURVIVOR > /root/survivor")
        status, managed = client.call_json("POST", "/vms/create", {
            "cpus": 4, "ram_mb": 12288,
            "managed": {"request_id": request_id, "ownership_token": capability},
        }, timeout=90)
        assert status == 200, managed
        assert execute(client, managed["id"], "printf MANAGED_NETWORK_READY") == "MANAGED_NETWORK_READY"
        status, shared = client.call_json("POST", "/networks", {"name": "shared"})
        assert status == 201, shared
        named_member = join(client, shared["id"], named["id"])
        managed_member = join(client, shared["id"], managed["id"])
        shared_switches = switches(service)
        assert len(shared_switches) == 1
        status, sole = client.call_json("POST", "/networks", {"name": "sole-managed"})
        assert status == 201, sole
        sole_member = join(client, sole["id"], managed["id"])
        before_switches = switches(service)
        assert len(before_switches) == 2
        assert sole_member["address"] in execute(client, managed["id"], "ip -o -4 addr show")
        assert managed_member["address"] in execute(client, managed["id"], "ip -o -4 addr show")
        status, closed = client.call_json("POST", f"/vms/managed/{request_id}/close", {
            "ownership_token": capability, "outcome": "completed",
        }, timeout=90)
        assert status == 200 and closed["state"] == "closed", closed
        retired = [process for process in before_switches if process.pid != shared_switches[0].pid]
        assert_retired(retired)
        assert [process.pid for process in switches(service)] == [shared_switches[0].pid]
        assert client.get_raw(f"/networks/{sole['id']}") == 404
        assert client.get(f"/networks/{shared['id']}")["members"] == [named_member]
        assert [network["id"] for network in client.get("/networks")["networks"]] == [shared["id"]]
        assert execute(client, named["id"], "cat /root/survivor") == "NAMED_SURVIVOR"
        assert named_member["address"] in execute(client, named["id"], "ip -o -4 addr show")
        status, deleted = client.call_json("DELETE", f"/vms/{named['id']}/delete")
        assert status == 200, deleted
        assert_retired(shared_switches)
        assert client.get("/networks")["networks"] == []
    finally:
        service.stop()


@pytest.mark.integration
def test_lost_create_reply_can_be_claimed_and_concurrently_closed_without_replay() -> None:
    service = ServiceInstance()
    request_id, capability = str(uuid.uuid4()), secrets.token_hex(32)
    try:
        service.start()
        client = gateway(service)
        # Deliberately discard the original create response body. Recovery uses
        # the caller's identity/capability, never another create attempt.
        connection = http.client.HTTPConnection("127.0.0.1", int(client.base_url.rsplit(":", 1)[1]), timeout=90)
        try:
            connection.request("POST", "/vms/create", json.dumps({
                "cpus": 4, "ram_mb": 12288,
                "managed": {"request_id": request_id, "ownership_token": capability},
            }), headers={"Content-Type": "application/json", "Authorization": f"Bearer {client.token}"})
            response = connection.getresponse()
            assert response.status == 200
            response.close()
        finally:
            connection.close()
        status, recovered = client.call_json("POST", f"/vms/managed/{request_id}/claim", {
            "ownership_token": capability,
        })
        assert status == 200 and recovered["state"] == "active", recovered
        session_id = recovered["session"]["id"]
        generation = recovered["ownership"]["generation"]
        assert len(client.get("/vms/list")["sandboxes"]) == 1
        assert execute(client, session_id, "printf RECOVERED_NATIVE") == "RECOVERED_NATIVE"
        directory = vm_session_dir(service.tmp_dir, client, session_id)
        assert service.proc is not None
        native = [process for process in psutil.Process(service.proc.pid).children()
                  if process.name() == "capsem-process"]
        assert len(native) == 1
        native += native[0].children(recursive=True)

        def close() -> tuple[int, object]:
            return TcpHttpClient(client.base_url, client.token).call_json(
                "POST", f"/vms/managed/{request_id}/close",
                {"ownership_token": capability, "outcome": "cancelled"}, timeout=90,
            )

        with ThreadPoolExecutor(max_workers=4) as workers:
            replies = list(workers.map(lambda _: close(), range(4)))
        first = replies[0][1]
        assert all(status == 200 and payload == first for status, payload in replies), replies
        assert first["request_id"] == request_id and first["generation"] == generation
        assert first["state"] == "closed" and first["session_id"] == session_id
        assert_retired(native)
        assert not directory.exists()
        assert client.get("/vms/list")["sandboxes"] == []
    finally:
        service.stop()
