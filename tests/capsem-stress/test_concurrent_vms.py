"""Concurrent VM creation, execution, and cleanup."""

import uuid

import pytest

from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = pytest.mark.stress


def test_create_five_vms():
    """Create 5 VMs, exec in all, delete all."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()
    vms = []

    try:
        for i in range(5):
            name = f"stress-{i}-{uuid.uuid4().hex[:6]}"
            resp = client.post("/provision", {"name": name, "ram_mb": 1024, "cpus": 1})
            assert resp is not None, f"VM {i} provision failed"
            vms.append(name)

        # Wait for all to be exec-ready
        for name in vms:
            assert wait_exec_ready(client, name, timeout=60), f"VM {name} never exec-ready"

        # Exec in each, verify isolation
        for i, name in enumerate(vms):
            resp = client.post(f"/exec/{name}", {"command": f"echo vm-{i}"})
            assert f"vm-{i}" in resp.get("stdout", "")

        # All in list
        list_resp = client.get("/list")
        ids = [s["id"] for s in list_resp["sandboxes"]]
        for name in vms:
            assert name in ids

    finally:
        for name in vms:
            try:
                client.delete(f"/delete/{name}")
            except Exception:
                pass
        svc.stop()


def test_rapid_create_delete():
    """Create and immediately delete 10 VMs in sequence."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()

    try:
        for i in range(10):
            name = f"rapid-{i}-{uuid.uuid4().hex[:6]}"
            resp = client.post("/provision", {"name": name, "ram_mb": 512, "cpus": 1})
            assert resp is not None, f"Cycle {i} provision failed"
            client.delete(f"/delete/{name}")

        # After all cycles, list should be clean (or only have pre-existing VMs)
        list_resp = client.get("/list")
        ids = [s["id"] for s in list_resp["sandboxes"]]
        rapid_ids = [i for i in ids if i.startswith("rapid-")]
        assert len(rapid_ids) == 0, f"Leaked VMs: {rapid_ids}"

    finally:
        svc.stop()
