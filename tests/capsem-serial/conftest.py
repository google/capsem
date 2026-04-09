"""Shared fixtures for serial console and boot timing tests."""

import uuid

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = pytest.mark.serial


@pytest.fixture(scope="session")
def serial_env():
    """Start service, boot VM, wait for exec-ready."""
    svc = ServiceInstance()
    svc.start()

    client = svc.client()
    vm_name = f"serial-{uuid.uuid4().hex[:8]}"
    client.post("/provision", {"name": vm_name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})

    if not wait_exec_ready(client, vm_name):
        svc.stop()
        pytest.fail(f"VM {vm_name} never became exec-ready")

    yield client, vm_name

    try:
        client.delete(f"/delete/{vm_name}")
    except Exception:
        pass
    svc.stop()
