"""Verify service is fully functional after recovering from bad state."""

import signal
import uuid

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = pytest.mark.recovery


def test_service_healthy_after_orphan_cleanup():
    """After recovering from orphaned VMs, service can create new VMs normally."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()

    try:
        # Create a VM, then kill the service
        name1 = f"victim-{uuid.uuid4().hex[:8]}"
        client.post("/provision", {"name": name1, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
        wait_exec_ready(client, name1, timeout=EXEC_READY_TIMEOUT)

        # Kill service (simulates crash)
        svc.proc.kill()
        svc.proc.wait()

        # Restart on same run dir
        svc2 = ServiceInstance()
        svc2.tmp_dir = svc.tmp_dir
        svc2.uds_path = svc.uds_path

        try:
            svc2.start()
            client2 = svc2.client()

            # Clean up orphan
            try:
                client2.delete(f"/delete/{name1}")
            except Exception:
                pass

            # Create a NEW VM -- service should be fully functional
            name2 = f"fresh-{uuid.uuid4().hex[:8]}"
            resp = client2.post("/provision", {"name": name2, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
            assert resp is not None, "Should create VM after recovery"

            assert wait_exec_ready(client2, name2, timeout=EXEC_READY_TIMEOUT), \
                "New VM should become exec-ready after recovery"

            exec_resp = client2.post(f"/exec/{name2}", {"command": "echo recovered"})
            assert "recovered" in exec_resp.get("stdout", ""), "Exec should work after recovery"

            client2.delete(f"/delete/{name2}")

        finally:
            svc2.stop()

    finally:
        try:
            svc.stop()
        except Exception:
            pass
