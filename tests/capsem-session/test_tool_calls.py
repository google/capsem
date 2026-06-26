"""Verify tool calls are recorded in the unified ledger."""

import pytest

pytestmark = pytest.mark.session


def test_mcp_calls_table_absent(session_db):
    tables = [
        r[0]
        for r in session_db.execute(
            "SELECT name FROM sqlite_master WHERE type='table'"
        ).fetchall()
    ]
    assert "mcp_calls" not in tables


def test_tool_calls_schema_has_mcp_origin_fields(session_db):
    cols = [r[1] for r in session_db.execute("PRAGMA table_info(tool_calls)").fetchall()]
    for required in ["origin", "server_name", "method", "tool_name", "decision", "duration_ms"]:
        assert required in cols, f"Missing column: {required}"


def test_tool_calls_have_timestamp(session_db):
    rows = session_db.execute("SELECT timestamp FROM tool_calls LIMIT 5").fetchall()
    for row in rows:
        assert row["timestamp"], "tool_call timestamp should not be empty"
