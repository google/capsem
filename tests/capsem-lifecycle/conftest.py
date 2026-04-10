"""Shared fixtures for VM lifecycle integration tests."""

import pytest

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
