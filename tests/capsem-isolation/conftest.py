"""Shared fixtures for multi-VM isolation tests."""

import uuid

import pytest

from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = pytest.mark.isolation


@pytest.fixture(scope="session")
def multi_vm_env():
    """Start service with 2 VMs (vm_a, vm_b), both exec-ready."""
    svc = ServiceInstance()
    svc.start()

    client = svc.client()

    vm_a = f"iso-a-{uuid.uuid4().hex[:8]}"
    vm_b = f"iso-b-{uuid.uuid4().hex[:8]}"
    client.post("/provision", {"name": vm_a, "ram_mb": 2048, "cpus": 2})
    client.post("/provision", {"name": vm_b, "ram_mb": 2048, "cpus": 2})

    assert wait_exec_ready(client, vm_a), f"VM {vm_a} never exec-ready"
    assert wait_exec_ready(client, vm_b), f"VM {vm_b} never exec-ready"

    yield client, vm_a, vm_b, svc.tmp_dir

    for vm in (vm_a, vm_b):
        try:
            client.delete(f"/delete/{vm}")
        except Exception:
            pass
    svc.stop()
