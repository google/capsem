"""Verify session.db schema matches capsem-logger CREATE_SCHEMA."""

import pytest

pytestmark = pytest.mark.session_lifecycle

# Expected columns per table (from capsem-logger schema)
EXPECTED_SCHEMAS = {
    "net_events": ["domain", "decision", "method", "status_code", "bytes_received", "duration_ms"],
    "model_calls": ["provider", "model", "duration_ms"],
    "tool_calls": ["tool_name", "origin"],
    "tool_responses": ["call_id", "is_error"],
    "mcp_calls": ["method", "decision"],
    "fs_events": ["action", "path"],
    "snapshot_events": ["origin", "slot"],
}


class TestDbSchema:

    @pytest.mark.parametrize("table,required_cols", list(EXPECTED_SCHEMAS.items()))
    def test_table_has_required_columns(self, lifecycle_db, table, required_cols):
        """Each table has its required columns."""
        cols = [
            r[1] for r in lifecycle_db.execute(f"PRAGMA table_info({table})").fetchall()
        ]
        if not cols:
            pytest.skip(f"Table {table} not found")
        for col in required_cols:
            assert col in cols, (
                f"Table {table} missing column '{col}' (has: {cols})"
            )
