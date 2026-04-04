"""Verify mcp_calls are logged when MCP operations occur."""

import pytest

pytestmark = pytest.mark.session


def test_mcp_calls_table_exists(session_db):
    tables = [r[0] for r in session_db.execute(
        "SELECT name FROM sqlite_master WHERE type='table'"
    ).fetchall()]
    assert "mcp_calls" in tables


def test_mcp_calls_schema(session_db):
    cols = [r[1] for r in session_db.execute("PRAGMA table_info(mcp_calls)").fetchall()]
    for required in ["method", "tool_name", "decision", "duration_ms"]:
        assert required in cols, f"Missing column: {required}"


def test_mcp_calls_have_timestamp(session_db):
    rows = session_db.execute("SELECT timestamp FROM mcp_calls LIMIT 5").fetchall()
    for row in rows:
        assert row["timestamp"], "mcp_call timestamp should not be empty"
