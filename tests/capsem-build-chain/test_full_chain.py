"""Full build chain E2E: build -> sign -> pack -> manifest -> boot VM -> exec -> delete."""

import uuid

import pytest

from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = pytest.mark.build_chain


def test_full_chain_boot_exec_delete(signed_binaries):
    """End-to-end: build + sign + boot VM + exec + delete."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()
    name = f"chain-{uuid.uuid4().hex[:8]}"

    try:
        resp = client.post("/provision", {"name": name, "ram_mb": 2048, "cpus": 2})
        assert resp is not None, f"Provision failed: {resp}"

        assert wait_exec_ready(client, name, timeout=30), (
            f"VM {name} never became exec-ready"
        )

        resp = client.post(f"/exec/{name}", {"command": "echo chain-works"})
        assert resp is not None
        assert "chain-works" in resp.get("stdout", ""), (
            f"Expected 'chain-works' in stdout, got: {resp}"
        )

        client.delete(f"/delete/{name}")

        # Verify deleted
        list_resp = client.get("/list")
        ids = [s["id"] for s in list_resp["sandboxes"]]
        assert name not in ids, f"VM {name} still in list after delete"

    finally:
        try:
            client.delete(f"/delete/{name}")
        except Exception:
            pass
        svc.stop()
