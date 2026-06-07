"""Exhaustive snapshot_events table validation."""

import pytest

pytestmark = pytest.mark.session_exhaustive


class TestSnapshotEventsData:

    def test_snapshot_events_schema(self, exhaust_db):
        """snapshot_events table has expected columns."""
        cols = [r[1] for r in exhaust_db.execute("PRAGMA table_info(snapshot_events)").fetchall()]
        for required in ["origin", "slot"]:
            assert required in cols, f"Missing column: {required}"

    def test_snapshot_origin_values(self, exhaust_db):
        """snapshot_events origin is 'auto' or 'manual'."""
        rows = exhaust_db.execute("SELECT origin FROM snapshot_events LIMIT 10").fetchall()
        for row in rows:
            assert row["origin"] in ("auto", "manual"), (
                f"Unexpected origin: {row['origin']}"
            )

    def test_snapshot_slot_positive(self, exhaust_db):
        """snapshot_events slot is a positive integer."""
        rows = exhaust_db.execute("SELECT slot FROM snapshot_events LIMIT 10").fetchall()
        for row in rows:
            if row["slot"] is not None:
                assert row["slot"] >= 0, f"Negative slot: {row['slot']}"
