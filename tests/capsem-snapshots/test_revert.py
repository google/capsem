"""Snapshot revert: restore files from a checkpoint."""

import pytest

pytestmark = pytest.mark.snapshot


def test_revert_restores_original_content():
    """Write file -> snapshot -> modify -> revert -> original restored."""
    pytest.skip("Requires snapshot MCP tools exposed via service -- see implementation-tasks.md")


def test_revert_deleted_file():
    """Revert recreates a file that was deleted after the snapshot."""
    pytest.skip("Requires snapshot MCP tools exposed via service -- see implementation-tasks.md")


def test_revert_nonexistent_snapshot():
    """Reverting to a nonexistent slot should error."""
    pytest.skip("Requires snapshot MCP tools exposed via service -- see implementation-tasks.md")
