"""Exhaustive tool_calls and tool_responses table validation."""

import pytest

pytestmark = pytest.mark.session_exhaustive


class TestToolCallsData:

    def test_tool_calls_schema(self, exhaust_db):
        """tool_calls table has expected columns."""
        cols = [r[1] for r in exhaust_db.execute("PRAGMA table_info(tool_calls)").fetchall()]
        for required in ["tool_name", "origin"]:
            assert required in cols, f"Missing column: {required}"

    def test_tool_calls_origin_values(self, exhaust_db):
        """tool_calls origin is 'native' or 'mcp'."""
        rows = exhaust_db.execute("SELECT origin FROM tool_calls LIMIT 10").fetchall()
        for row in rows:
            assert row["origin"] in ("native", "mcp"), (
                f"Unexpected origin: {row['origin']}"
            )


class TestToolResponsesData:

    def test_tool_responses_schema(self, exhaust_db):
        """tool_responses table has expected columns."""
        cols = [r[1] for r in exhaust_db.execute("PRAGMA table_info(tool_responses)").fetchall()]
        for required in ["call_id", "is_error"]:
            assert required in cols, f"Missing column: {required}"

    def test_tool_response_error_flag(self, exhaust_db):
        """tool_responses is_error is 0 or 1."""
        rows = exhaust_db.execute("SELECT is_error FROM tool_responses LIMIT 10").fetchall()
        for row in rows:
            assert row["is_error"] in (0, 1), (
                f"is_error should be 0 or 1, got: {row['is_error']}"
            )

    def test_tool_response_has_matching_call(self, exhaust_db):
        """Every tool_response has a matching tool_calls.id."""
        orphans = exhaust_db.execute("""
            SELECT tr.call_id FROM tool_responses tr
            LEFT JOIN tool_calls tc ON tr.call_id = tc.id
            WHERE tc.id IS NULL
        """).fetchall()
        assert len(orphans) == 0, f"Orphaned tool responses: {[r['call_id'] for r in orphans]}"
