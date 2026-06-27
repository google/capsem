"""VM-A survives deletion of VM-B: files persist, exec still works."""

import uuid

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = pytest.mark.isolation


def test_resume_after_neighbor_delete():
    """Start A+B, write in A, delete B, verify A still works."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()

    vm_a = f"resume-a-{uuid.uuid4().hex[:8]}"
    vm_b = f"resume-b-{uuid.uuid4().hex[:8]}"

    try:
        create_a = client.post(
            "/vms/create", {"name": vm_a, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS}
        )
        create_b = client.post(
            "/vms/create", {"name": vm_b, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS}
        )
        vm_a_id = create_a["id"]
        vm_b_id = create_b["id"]

        assert wait_exec_ready(client, vm_a_id), "VM-A never exec-ready"
        assert wait_exec_ready(client, vm_b_id), "VM-B never exec-ready"

        # Write a file in VM-A
        client.post(f"/vms/{vm_a_id}/files/write", {
            "path": "/root/resume-test.txt",
            "content": "still-here",
        })

        # Delete VM-B
        client.delete(f"/vms/{vm_b_id}/delete")

        # VM-A file should still be there
        resp = client.post(f"/vms/{vm_a_id}/files/read", {"path": "/root/resume-test.txt"})
        assert resp.get("content") == "still-here"

        # VM-A exec should still work
        resp = client.post(f"/vms/{vm_a_id}/exec", {"command": "echo alive"})
        assert "alive" in resp.get("stdout", "")

        # VM-B should be gone from list
        list_resp = client.get("/vms/list")
        ids = [s["id"] for s in list_resp["sandboxes"]]
        assert vm_b_id not in ids
        assert vm_a_id in ids

    finally:
        for vm in (locals().get("vm_a_id", vm_a), locals().get("vm_b_id", vm_b)):
            try:
                client.delete(f"/vms/{vm}/delete")
            except Exception:
                pass
        svc.stop()
