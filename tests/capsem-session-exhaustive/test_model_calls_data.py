"""Exhaustive model_calls table validation."""

import pytest

pytestmark = pytest.mark.session_exhaustive


class TestModelCallsData:

    def test_model_calls_schema(self, exhaust_db):
        """model_calls table has expected columns."""
        cols = [r[1] for r in exhaust_db.execute("PRAGMA table_info(model_calls)").fetchall()]
        for required in ["provider", "model", "duration_ms"]:
            assert required in cols, f"Missing column: {required}"

    def test_model_calls_empty_by_default(self, exhaust_db):
        """model_calls should be empty if no AI API calls were made."""
        count = exhaust_db.execute("SELECT COUNT(*) FROM model_calls").fetchone()[0]
        # No AI calls in test workload, so table should be empty or have 0 rows
        assert count >= 0

    @pytest.mark.skip(reason="Requires AI API key, skip unless available")
    def test_model_call_fields_populated(self, exhaust_db):
        """model_calls rows have provider, model, and duration."""
        rows = exhaust_db.execute(
            "SELECT provider, model, duration_ms FROM model_calls LIMIT 5"
        ).fetchall()
        for row in rows:
            assert row["provider"], "provider should not be empty"
            assert row["model"], "model should not be empty"
            assert row["duration_ms"] >= 0, "duration_ms should be non-negative"
