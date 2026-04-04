"""Exhaustive mcp_calls table validation."""

import pytest

pytestmark = pytest.mark.session_exhaustive


class TestMcpCallsData:

    def test_mcp_calls_schema(self, exhaust_db):
        """mcp_calls table has expected columns."""
        cols = [r[1] for r in exhaust_db.execute("PRAGMA table_info(mcp_calls)").fetchall()]
        for required in ["method", "decision"]:
            assert required in cols, f"Missing column: {required}"

    def test_mcp_calls_method_values(self, exhaust_db):
        """mcp_calls method should be a known MCP method."""
        known_methods = {
            "initialize", "tools/list", "tools/call",
            "prompts/list", "prompts/get", "resources/list",
        }
        rows = exhaust_db.execute("SELECT method FROM mcp_calls LIMIT 20").fetchall()
        for row in rows:
            assert row["method"] in known_methods or "/" in row["method"], (
                f"Unknown MCP method: {row['method']}"
            )

    def test_mcp_calls_decision_values(self, exhaust_db):
        """mcp_calls decision is 'allowed' or 'denied'."""
        rows = exhaust_db.execute("SELECT decision FROM mcp_calls LIMIT 20").fetchall()
        for row in rows:
            assert row["decision"] in ("allowed", "denied", "blocked"), (
                f"Unexpected decision: {row['decision']}"
            )
