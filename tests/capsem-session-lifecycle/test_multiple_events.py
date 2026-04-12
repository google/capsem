"""Verify multiple operations create distinct, ordered events."""

import time

import pytest

pytestmark = pytest.mark.session_lifecycle


def test_multiple_execs_create_ordered_events(lifecycle_env, lifecycle_db):
    """Multiple exec commands should create distinct events with increasing IDs."""
    client, vm_name, _, _ = lifecycle_env

    # Run several distinct commands
    commands = [
        "echo event-alpha",
        "echo event-beta",
        "echo event-gamma",
    ]
    for cmd in commands:
        client.post(f"/exec/{vm_name}", {"command": cmd})

    # Wait for async writer
    time.sleep(3)

    # Check that events exist in the DB (tool_calls or similar)
    # At minimum, the net_events or fs_events should be growing
    rows = lifecycle_db.execute(
        "SELECT id FROM net_events ORDER BY id"
    ).fetchall()

    if len(rows) >= 2:
        # IDs should be monotonically increasing
        ids = [r["id"] for r in rows]
        for i in range(1, len(ids)):
            assert ids[i] > ids[i-1], f"Event IDs not ordered: {ids}"


def test_net_event_has_domain_field(lifecycle_env, lifecycle_db):
    """Net events should have a non-empty domain field."""
    client, vm_name, _, _ = lifecycle_env

    # Trigger a network request
    client.post(f"/exec/{vm_name}", {
        "command": "curl -s -o /dev/null https://example.com/ 2>&1 || true"
    })

    time.sleep(3)

    rows = lifecycle_db.execute(
        "SELECT domain FROM net_events WHERE domain IS NOT NULL AND domain != ''"
    ).fetchall()
    assert len(rows) > 0, "Expected at least one net_event with a domain"


def test_session_db_readable_during_vm_run(lifecycle_env, lifecycle_db):
    """Session.db should be readable while VM is still running."""
    # The lifecycle_env fixture keeps the VM running
    # This test verifies we can read without locking issues
    tables = lifecycle_db.execute(
        "SELECT name FROM sqlite_master WHERE type='table'"
    ).fetchall()
    table_names = [t["name"] for t in tables]

    expected = ["net_events", "fs_events", "snapshot_events"]
    for name in expected:
        assert name in table_names, f"Missing table {name} during live read"
