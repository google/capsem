"""Shared fixtures for VM cleanup verification tests."""

import pytest

from helpers.service import ServiceInstance

pytestmark = pytest.mark.cleanup


@pytest.fixture(scope="session")
def cleanup_env():
    """Start service for cleanup tests."""
    svc = ServiceInstance()
    svc.start()
    yield svc
    svc.stop()
