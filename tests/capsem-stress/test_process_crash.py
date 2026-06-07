"""Service recovery after capsem-process crash."""

import os
import signal
import time
import uuid

import pytest

from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = pytest.mark.stress


def test_service_survives_process_kill():
    """kill -9 a capsem-process; service should detect it and allow new VMs."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()

    try:
        # Create a VM
        name = f"crash-{uuid.uuid4().hex[:8]}"
        client.post("/provision", {"name": name, "ram_mb": 1024, "cpus": 1})

        # Get its PID from info
        info = client.get(f"/info/{name}")
        pid = info.get("pid", 0) if info else 0

        if pid > 0:
            # Kill the process
            try:
                os.kill(pid, signal.SIGKILL)
                time.sleep(2)
            except ProcessLookupError:
                pass

        # Service should still be alive
        list_resp = client.get("/list")
        assert list_resp is not None, "Service died after process kill"

        # Clean up the dead VM
        try:
            client.delete(f"/delete/{name}")
        except Exception:
            pass

        # Should be able to create a new VM
        name2 = f"after-crash-{uuid.uuid4().hex[:8]}"
        resp = client.post("/provision", {"name": name2, "ram_mb": 1024, "cpus": 1})
        assert resp is not None, "Could not create VM after process crash"
        client.delete(f"/delete/{name2}")

    finally:
        svc.stop()
