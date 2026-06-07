"""Manual (named) snapshot create, list, delete."""

import pytest

pytestmark = pytest.mark.snapshot

# NOTE: Manual snapshot operations require MCP tools (snapshots_create,
# snapshots_list, snapshots_delete) exposed via the service API or MCP.
# These tests document the expected behavior. Until the service exposes
# snapshot tools, they will skip or fail -- that's intentional (TDD).
# See implementation-tasks.md.


def test_create_named_snapshot():
    """Create a named snapshot via MCP tool and verify it appears in list."""
    pytest.skip("Requires snapshot MCP tools exposed via service -- see implementation-tasks.md")


def test_delete_named_snapshot():
    """Delete a named snapshot and verify it's gone from list."""
    pytest.skip("Requires snapshot MCP tools exposed via service -- see implementation-tasks.md")


def test_max_manual_slots_enforced():
    """Creating more than max_manual named snapshots should fail."""
    pytest.skip("Requires snapshot MCP tools exposed via service -- see implementation-tasks.md")


def test_snapshot_has_blake3_hash():
    """Manual snapshots should include a blake3 hash."""
    pytest.skip("Requires snapshot MCP tools exposed via service -- see implementation-tasks.md")
