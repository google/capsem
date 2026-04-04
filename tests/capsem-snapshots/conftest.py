"""Shared fixtures for snapshot lifecycle tests."""

import pytest

pytestmark = pytest.mark.snapshot

# Reuses session_env pattern from capsem-session.
# Will be expanded when snapshot MCP tools are available via service API.
