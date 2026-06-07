"""Verify file writes generate fs_events in session.db."""

import time

import pytest

pytestmark = pytest.mark.session_lifecycle


def test_file_write_creates_fs_event(lifecycle_env, lifecycle_db):
    """Writing a file via API should appear in fs_events."""
    client, vm_name, _, _ = lifecycle_env

    # Write a file via the file API
    client.post(f"/write-file/{vm_name}", {
        "path": "/capsem/workspace/test-lifecycle.txt",
        "content": "lifecycle test data",
    })

    # Wait for async writer to flush
    time.sleep(3)

    rows = lifecycle_db.execute(
        "SELECT action, path FROM fs_events"
    ).fetchall()
    # Host file monitor should have captured the write
    assert len(rows) > 0, "Expected at least one fs_event from file write"
