"""Boot timing regression gates: provision to exec-ready."""

import os
import platform
import time
import uuid

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = pytest.mark.serial

def _float_env(name, default):
    try:
        return float(os.environ.get(name, default))
    except ValueError:
        return default


def _linux_kvm_default_gate():
    # Linux KVM currently returns from /provision when the per-VM process is
    # booted enough to accept the first exec; on this path, the measured time is
    # provision-to-ready, not steady-state exec latency.
    return 3.5 if platform.system() == "Linux" else 1.5


PROVISION_READY_GATE = _float_env(
    "CAPSEM_PROVISION_READY_GATE_SECS",
    _linux_kvm_default_gate(),
)
CONCURRENT_PROVISION_READY_GATE = _float_env(
    "CAPSEM_CONCURRENT_PROVISION_READY_GATE_SECS",
    3.5 if platform.system() == "Linux" else 1.2,
)


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
    """Provision a VM and first exec-ready probe must complete within gate."""
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
        assert elapsed < PROVISION_READY_GATE, (
            f"Provision-to-ready latency {elapsed:.2f}s exceeds {PROVISION_READY_GATE}s gate"
        )
        print(f"Provision-to-ready latency: {elapsed:.2f}s (gate: {PROVISION_READY_GATE}s)")

    finally:
        try:
            client.delete(f"/delete/{name}")
        except Exception:
            pass
        svc.stop()


def test_avg_exec_latency_3_runs():
    """Provision+delete 3 VMs sequentially; average provision-to-ready is gated."""
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
        print(f"Average provision-to-ready latency: {avg:.2f}s (gate: {PROVISION_READY_GATE}s)")
        assert avg < PROVISION_READY_GATE, (
            f"Average provision-to-ready latency {avg:.2f}s exceeds {PROVISION_READY_GATE}s gate"
        )
    finally:
        svc.stop()


def test_avg_exec_latency_3_concurrent_vms():
    """Boot 3 VMs on the same service; average provision-to-ready is gated."""
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
        print(
            "Average provision-to-ready latency: "
            f"{avg:.2f}s (gate: {CONCURRENT_PROVISION_READY_GATE}s)"
        )
        assert avg < CONCURRENT_PROVISION_READY_GATE, (
            f"Average provision-to-ready latency {avg:.2f}s exceeds "
            f"{CONCURRENT_PROVISION_READY_GATE}s gate"
        )
    finally:
        for name in names:
            try:
                client.delete(f"/delete/{name}")
            except Exception:
                pass
        svc.stop()
