"""Exhaustive unified tool_calls table validation."""

import pytest

pytestmark = pytest.mark.session_exhaustive


class TestToolCallsData:

    def test_mcp_calls_table_absent(self, exhaust_db):
        """mcp_calls is retired; all tool evidence lives in tool_calls."""
        rows = exhaust_db.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='mcp_calls'"
        ).fetchall()
        assert rows == []

    def test_tool_calls_schema(self, exhaust_db):
        """tool_calls table has expected unified columns."""
        cols = [r[1] for r in exhaust_db.execute("PRAGMA table_info(tool_calls)").fetchall()]
        for required in ["origin", "server_name", "method", "tool_name", "decision"]:
            assert required in cols, f"Missing column: {required}"

    def test_mcp_origin_tool_call_method_values(self, exhaust_db):
        """MCP-origin tool_calls method should be a known MCP method."""
        known_methods = {
            "initialize", "tools/list", "tools/call",
            "prompts/list", "prompts/get", "resources/list",
        }
        rows = exhaust_db.execute(
            "SELECT method FROM tool_calls WHERE origin = 'mcp' LIMIT 20"
        ).fetchall()
        for row in rows:
            assert row["method"] in known_methods or "/" in row["method"], (
                f"Unknown MCP method: {row['method']}"
            )

    def test_tool_call_decision_values(self, exhaust_db):
        """tool_calls decision is canonical."""
        rows = exhaust_db.execute("SELECT decision FROM tool_calls LIMIT 20").fetchall()
        for row in rows:
            assert row["decision"] in ("allowed", "denied", "blocked"), (
                f"Unexpected decision: {row['decision']}"
            )
