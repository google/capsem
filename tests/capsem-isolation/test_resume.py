"""VM-A survives deletion of VM-B: files persist, exec still works."""

import uuid

import pytest

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
        client.post("/provision", {"name": vm_a, "ram_mb": 2048, "cpus": 2})
        client.post("/provision", {"name": vm_b, "ram_mb": 2048, "cpus": 2})

        assert wait_exec_ready(client, vm_a), f"VM-A never exec-ready"
        assert wait_exec_ready(client, vm_b), f"VM-B never exec-ready"

        # Write a file in VM-A
        client.post(f"/write_file/{vm_a}", {
            "path": "/tmp/resume-test.txt",
            "content": "still-here",
        })

        # Delete VM-B
        client.delete(f"/delete/{vm_b}")

        # VM-A file should still be there
        resp = client.post(f"/read_file/{vm_a}", {"path": "/tmp/resume-test.txt"})
        assert resp.get("content") == "still-here"

        # VM-A exec should still work
        resp = client.post(f"/exec/{vm_a}", {"command": "echo alive"})
        assert "alive" in resp.get("stdout", "")

        # VM-B should be gone from list
        list_resp = client.get("/list")
        ids = [s["id"] for s in list_resp["sandboxes"]]
        assert vm_b not in ids
        assert vm_a in ids

    finally:
        for vm in (vm_a, vm_b):
            try:
                client.delete(f"/delete/{vm}")
            except Exception:
                pass
        svc.stop()
