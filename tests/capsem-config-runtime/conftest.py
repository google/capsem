"""Shared fixtures for config runtime tests.

Verifies that config values (CPU, RAM, blocked domains) are applied at runtime.
"""

import pytest

from helpers.service import ServiceInstance

pytestmark = pytest.mark.config_runtime


@pytest.fixture(scope="session")
def config_svc():
    """Start service for config runtime tests."""
    svc = ServiceInstance()
    svc.start()
    yield svc
    svc.stop()
