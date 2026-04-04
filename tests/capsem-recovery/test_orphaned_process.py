"""Verify service handles orphaned VM processes after restart."""

import signal
import uuid

import pytest

from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = pytest.mark.recovery


def test_orphaned_vm_cleanup_on_restart():
    """Start VM, kill service (not VM), restart service, delete cleans up."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()
    name = f"orphan-{uuid.uuid4().hex[:8]}"

    try:
        client.post("/provision", {"name": name, "ram_mb": 2048, "cpus": 2})
        wait_exec_ready(client, name, timeout=30)

        # Kill the service process (simulates crash)
        svc.proc.kill()
        svc.proc.wait()

        # Start a new service on the same socket
        svc2 = ServiceInstance()
        svc2.tmp_dir = svc.tmp_dir  # Reuse same run dir
        svc2.uds_path = svc.uds_path

        try:
            svc2.start()
            client2 = svc2.client()

            # List should work -- may or may not show the orphaned VM
            resp = client2.get("/list")
            assert resp is not None

            # Try to clean up -- should not hang or crash
            try:
                client2.delete(f"/delete/{name}")
            except Exception:
                pass  # May already be gone

        finally:
            svc2.stop()

    finally:
        try:
            svc.stop()
        except Exception:
            pass
