"""Verify snapshot_events are logged when snapshots are taken."""

import pytest

pytestmark = pytest.mark.session


def test_snapshot_events_table_exists(session_db):
    tables = [r[0] for r in session_db.execute(
        "SELECT name FROM sqlite_master WHERE type='table'"
    ).fetchall()]
    assert "snapshot_events" in tables


def test_snapshot_events_schema(session_db):
    cols = [r[1] for r in session_db.execute("PRAGMA table_info(snapshot_events)").fetchall()]
    for required in ["slot", "origin", "name", "files_count",
                     "start_fs_event_id", "stop_fs_event_id"]:
        assert required in cols, f"Missing column: {required}"


def test_snapshot_events_have_timestamp(session_db):
    rows = session_db.execute("SELECT timestamp FROM snapshot_events LIMIT 5").fetchall()
    for row in rows:
        assert row["timestamp"], "snapshot_event timestamp should not be empty"
