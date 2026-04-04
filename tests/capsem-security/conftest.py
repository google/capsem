"""Shared fixtures for security invariant tests.

Provides a VM fixture for in-guest security checks via exec.
"""

import pytest

# Reuses service_env + client + fresh_vm from capsem-service conftest.
# Security tests exec commands inside the guest to verify invariants.

pytestmark = pytest.mark.security
