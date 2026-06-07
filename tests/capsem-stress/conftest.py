"""Shared fixtures for stress and load tests."""

import pytest

# Reuses service_env + client from capsem-service conftest.
# Stress tests verify concurrency, resource cleanup, crash recovery.

pytestmark = pytest.mark.stress
