"""Shared fixtures for security invariant tests.

Provides a VM fixture for in-guest security checks via exec.
"""

import pytest

from helpers.service import ServiceInstance

pytestmark = pytest.mark.security


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
