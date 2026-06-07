"""Exhaustive fs_events table validation."""

import pytest

pytestmark = pytest.mark.session_exhaustive


class TestFsEventsData:

    def test_fs_events_schema(self, exhaust_db):
        """fs_events table has expected columns."""
        cols = [r[1] for r in exhaust_db.execute("PRAGMA table_info(fs_events)").fetchall()]
        for required in ["action", "path"]:
            assert required in cols, f"Missing column: {required}"

    def test_fs_event_action_values(self, exhaust_db):
        """fs_events action is a known filesystem action."""
        known_actions = {"created", "modified", "deleted", "renamed", "read"}
        rows = exhaust_db.execute("SELECT action FROM fs_events LIMIT 20").fetchall()
        for row in rows:
            assert row["action"] in known_actions, (
                f"Unknown fs action: {row['action']}"
            )

    def test_fs_event_has_path(self, exhaust_db):
        """fs_events rows have a non-empty path."""
        rows = exhaust_db.execute("SELECT path FROM fs_events LIMIT 20").fetchall()
        for row in rows:
            assert row["path"], "path should not be empty"

    def test_fs_event_has_timestamp(self, exhaust_db):
        """fs_events rows have ISO 8601 timestamps."""
        rows = exhaust_db.execute("SELECT timestamp FROM fs_events LIMIT 10").fetchall()
        for row in rows:
            ts = row["timestamp"]
            if ts:
                assert "T" in ts or "-" in ts, f"Timestamp not ISO 8601: {ts}"
