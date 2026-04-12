"""Verify VM names can be reused after deletion."""

import uuid

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = pytest.mark.stress


def test_create_delete_reuse_name():
    """Create, delete, and recreate a VM with the same name 3 times."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()
    name = f"reuse-{uuid.uuid4().hex[:8]}"

    try:
        for cycle in range(3):
            resp = client.post("/provision", {
                "name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS,
            })
            assert resp is not None, f"Cycle {cycle}: provision failed"

            assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT), \
                f"Cycle {cycle}: VM never exec-ready"

            exec_resp = client.post(f"/exec/{name}", {"command": f"echo cycle-{cycle}"})
            assert f"cycle-{cycle}" in exec_resp.get("stdout", ""), \
                f"Cycle {cycle}: exec output wrong"

            client.delete(f"/delete/{name}")

        # After all cycles, name should not appear in list
        list_resp = client.get("/list")
        ids = [s["id"] for s in list_resp.get("sandboxes", [])]
        assert name not in ids, f"VM {name} still in list after final delete"

    finally:
        try:
            client.delete(f"/delete/{name}")
        except Exception:
            pass
        svc.stop()


def test_service_healthy_after_mass_delete():
    """Create 5 VMs, delete all, service still responds to /list."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()
    vms = []

    try:
        for i in range(5):
            name = f"mass-{i}-{uuid.uuid4().hex[:6]}"
            client.post("/provision", {"name": name, "ram_mb": 512, "cpus": 1})
            vms.append(name)

        # Delete all
        for name in vms:
            client.delete(f"/delete/{name}")

        # Service should still be healthy
        resp = client.get("/list")
        assert resp is not None, "Service should respond after mass delete"
        ids = [s["id"] for s in resp.get("sandboxes", [])]
        mass_ids = [i for i in ids if i.startswith("mass-")]
        assert len(mass_ids) == 0, f"Leaked VMs: {mass_ids}"

    finally:
        for name in vms:
            try:
                client.delete(f"/delete/{name}")
            except Exception:
                pass
        svc.stop()
