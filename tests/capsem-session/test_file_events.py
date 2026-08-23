"""Verify the live fs_events schema and row integrity."""

import pytest

pytestmark = pytest.mark.session


def test_fs_events_table_exists(session_db):
    tables = [r[0] for r in session_db.execute(
        "SELECT name FROM sqlite_master WHERE type='table'"
    ).fetchall()]
    assert "fs_events" in tables


def test_file_event_has_path(session_db):
    """Every fs_event must have a non-empty path."""
    rows = session_db.execute("SELECT path FROM fs_events LIMIT 10").fetchall()
    for row in rows:
        assert row["path"], "fs_event path should not be empty"
