"""Shared fixtures for capsem-service HTTP API integration tests."""

import uuid

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import ServiceInstance, wait_exec_ready, vm_name

pytestmark = pytest.mark.integration


@pytest.fixture(scope="session")
def service_env():
    """Start a capsem-service on an isolated temp socket."""
    svc = ServiceInstance()
    svc.start()
    yield svc
    svc.stop()


@pytest.fixture
def client(service_env):
    """UDS HTTP client connected to the test service."""
    return service_env.client()


@pytest.fixture
def fresh_vm(client):
    """Factory: provision a VM, delete on teardown."""
    created = []

    def _create(prefix="svc", ram_mb=DEFAULT_RAM_MB, cpus=DEFAULT_CPUS):
        name = vm_name(prefix)
        resp = client.post("/provision", {"name": name, "ram_mb": ram_mb, "cpus": cpus})
        created.append(name)
        return name, resp

    yield _create

    for vm_id in created:
        try:
            client.delete(f"/delete/{vm_id}")
        except Exception:
            pass


@pytest.fixture(scope="module")
def ready_vm(service_env):
    """A single exec-ready VM that stays alive for the module. Yields (client, name)."""
    client = service_env.client()
    name = vm_name(service_env.__class__.__name__[:8])
    client.post("/provision", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
    assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT), f"VM {name} never exec-ready"
    yield client, name
    try:
        client.delete(f"/delete/{name}")
    except Exception:
        pass
