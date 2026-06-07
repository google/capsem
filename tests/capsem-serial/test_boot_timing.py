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


def test_avg_exec_latency_3_runs():
    """Provision+delete 3 VMs sequentially; average provision-to-exec must be < 1.5s."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()
    times = []

    try:
        for i in range(3):
            name = f"avg-{uuid.uuid4().hex[:8]}"
            start = time.time()
            client.post("/provision", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
            ready = wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)
            elapsed = time.time() - start
            assert ready, f"VM {i+1} never became exec-ready after {elapsed:.1f}s"
            times.append(elapsed)
            print(f"  run {i+1}: {elapsed:.2f}s")
            client.delete(f"/delete/{name}")

        avg = sum(times) / len(times)
        print(f"Average exec latency: {avg:.2f}s (gate: {EXEC_LATENCY_GATE}s)")
        assert avg < EXEC_LATENCY_GATE, (
            f"Average exec latency {avg:.2f}s exceeds {EXEC_LATENCY_GATE}s gate"
        )
    finally:
        svc.stop()


def test_avg_exec_latency_3_concurrent_vms():
    """Boot 3 VMs on the same service; average provision-to-exec < 1.2s."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()
    names = [f"conc-{uuid.uuid4().hex[:8]}" for _ in range(3)]
    times = []

    try:
        for i, name in enumerate(names):
            start = time.time()
            client.post("/provision", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
            ready = wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)
            elapsed = time.time() - start
            assert ready, f"VM {i+1} never became exec-ready after {elapsed:.1f}s"
            times.append(elapsed)
            print(f"  vm {i+1}: {elapsed:.2f}s")

        avg = sum(times) / len(times)
        print(f"Average exec latency: {avg:.2f}s (gate: 1.2s)")
        assert avg < 1.2, (
            f"Average exec latency {avg:.2f}s exceeds 1.2s gate"
        )
    finally:
        for name in names:
            try:
                client.delete(f"/delete/{name}")
            except Exception:
                pass
        svc.stop()
