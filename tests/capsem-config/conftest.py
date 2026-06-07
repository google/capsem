"""Shared fixtures for config obedience tests."""

import pytest

# Reuses service_env + client from capsem-service conftest.
# Config tests verify limits, resource bounds, per-VM settings, hot-reload.

pytestmark = pytest.mark.config
