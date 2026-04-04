"""Shared fixtures for capsem-service HTTP API integration tests."""

import uuid

import pytest

from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = pytest.mark.integration


def vm_name(prefix="test"):
    """Generate a unique VM name with the given prefix."""
    return f"{prefix}-{uuid.uuid4().hex[:8]}"


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

    def _create(prefix="svc", ram_mb=2048, cpus=2):
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
    client.post("/provision", {"name": name, "ram_mb": 2048, "cpus": 2})
    assert wait_exec_ready(client, name, timeout=30), f"VM {name} never exec-ready"
    yield client, name
    try:
        client.delete(f"/delete/{name}")
    except Exception:
        pass
