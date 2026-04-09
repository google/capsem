"""Boot timing regression gate: provision to exec-ready under 30s."""

import time
import uuid

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import ServiceInstance

pytestmark = pytest.mark.serial


def test_boot_under_30_seconds():
    """Provision a VM and measure time to exec-ready. Must be < 30s."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()
    name = f"timing-{uuid.uuid4().hex[:8]}"

    try:
        start = time.time()
        client.post("/provision", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})

        # Poll for exec-ready
        ready = False
        for _ in range(EXEC_READY_TIMEOUT):
            try:
                resp = client.post(f"/exec/{name}", {"command": "echo ready"})
                if resp and "ready" in resp.get("stdout", ""):
                    ready = True
                    break
            except Exception:
                pass
            time.sleep(1)

        elapsed = time.time() - start

        assert ready, f"VM never became exec-ready after {elapsed:.1f}s"
        assert elapsed < EXEC_READY_TIMEOUT, (
            f"Boot took {elapsed:.1f}s, exceeds {EXEC_READY_TIMEOUT}s regression gate"
        )

    finally:
        try:
            client.delete(f"/delete/{name}")
        except Exception:
            pass
        svc.stop()
