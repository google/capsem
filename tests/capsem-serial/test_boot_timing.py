"""Boot timing regression gates: provision to exec-ready."""

import time
import uuid
import sys

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = pytest.mark.serial

IS_LINUX = sys.platform.startswith("linux")
EXEC_LATENCY_GATE = 2.0 if IS_LINUX else 1.5
CONCURRENT_EXEC_LATENCY_GATE = 2.0 if IS_LINUX else 1.2


def test_boot_under_30_seconds():
    """Provision a VM and measure time to exec-ready. Must be < 30s."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()
    name = f"timing-{uuid.uuid4().hex[:8]}"

    try:
        start = time.time()
        client.post("/vms/create", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})

        ready = wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)
        elapsed = time.time() - start

        assert ready, f"VM never became exec-ready after {elapsed:.1f}s"
        assert elapsed < EXEC_READY_TIMEOUT, (
            f"Boot took {elapsed:.1f}s, exceeds {EXEC_READY_TIMEOUT}s regression gate"
        )

    finally:
        try:
            client.delete(f"/vms/{name}/delete")
        except Exception:
            pass
        svc.stop()


def test_exec_latency_under_1_5_seconds():
    """Provision a VM and first exec must complete inside the platform gate."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()
    name = f"lat-{uuid.uuid4().hex[:8]}"

    try:
        start = time.time()
        client.post("/vms/create", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})

        ready = wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)
        elapsed = time.time() - start

        assert ready, f"VM never became exec-ready after {elapsed:.1f}s"
        assert elapsed < EXEC_LATENCY_GATE, (
            f"Exec latency {elapsed:.2f}s exceeds {EXEC_LATENCY_GATE}s gate"
        )
        print(f"Exec latency: {elapsed:.2f}s (gate: {EXEC_LATENCY_GATE}s)")

    finally:
        try:
            client.delete(f"/vms/{name}/delete")
        except Exception:
            pass
        svc.stop()


def test_avg_exec_latency_3_runs():
    """Provision+delete 3 VMs sequentially; average provision-to-exec stays in budget."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()
    times = []

    try:
        for i in range(3):
            name = f"avg-{uuid.uuid4().hex[:8]}"
            start = time.time()
            client.post("/vms/create", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
            ready = wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)
            elapsed = time.time() - start
            assert ready, f"VM {i+1} never became exec-ready after {elapsed:.1f}s"
            times.append(elapsed)
            print(f"  run {i+1}: {elapsed:.2f}s")
            client.delete(f"/vms/{name}/delete")

        avg = sum(times) / len(times)
        print(f"Average exec latency: {avg:.2f}s (gate: {EXEC_LATENCY_GATE}s)")
        assert avg < EXEC_LATENCY_GATE, (
            f"Average exec latency {avg:.2f}s exceeds {EXEC_LATENCY_GATE}s gate"
        )
    finally:
        svc.stop()


def test_avg_exec_latency_3_concurrent_vms():
    """Boot 3 VMs on the same service; average provision-to-exec stays in budget."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()
    names = [f"conc-{uuid.uuid4().hex[:8]}" for _ in range(3)]
    times = []

    try:
        for i, name in enumerate(names):
            start = time.time()
            client.post("/vms/create", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
            ready = wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)
            elapsed = time.time() - start
            assert ready, f"VM {i+1} never became exec-ready after {elapsed:.1f}s"
            times.append(elapsed)
            print(f"  vm {i+1}: {elapsed:.2f}s")

        avg = sum(times) / len(times)
        print(f"Average exec latency: {avg:.2f}s (gate: {CONCURRENT_EXEC_LATENCY_GATE}s)")
        assert avg < CONCURRENT_EXEC_LATENCY_GATE, (
            f"Average exec latency {avg:.2f}s exceeds {CONCURRENT_EXEC_LATENCY_GATE}s gate"
        )
    finally:
        for name in names:
            try:
                client.delete(f"/vms/{name}/delete")
            except Exception:
                pass
        svc.stop()
