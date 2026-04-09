"""Boot timing regression gates: provision to exec-ready."""

import time
import uuid

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = pytest.mark.serial

EXEC_LATENCY_GATE = 1.5  # seconds -- provision to first exec must be under this


def test_boot_under_30_seconds():
    """Provision a VM and measure time to exec-ready. Must be < 30s."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()
    name = f"timing-{uuid.uuid4().hex[:8]}"

    try:
        start = time.time()
        client.post("/provision", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})

        ready = wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)
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


def test_exec_latency_under_1_5_seconds():
    """Provision a VM and first exec must complete in < 1.5s."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()
    name = f"lat-{uuid.uuid4().hex[:8]}"

    try:
        start = time.time()
        client.post("/provision", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})

        ready = wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)
        elapsed = time.time() - start

        assert ready, f"VM never became exec-ready after {elapsed:.1f}s"
        assert elapsed < EXEC_LATENCY_GATE, (
            f"Exec latency {elapsed:.2f}s exceeds {EXEC_LATENCY_GATE}s gate"
        )
        print(f"Exec latency: {elapsed:.2f}s (gate: {EXEC_LATENCY_GATE}s)")

    finally:
        try:
            client.delete(f"/delete/{name}")
        except Exception:
            pass
        svc.stop()
