"""Cross-table foreign key validation for session.db."""

import pytest

pytestmark = pytest.mark.session_exhaustive


class TestCrossTableForeignKeys:

    def test_tool_calls_model_call_fk(self, exhaust_db):
        """tool_calls.model_call_id references a valid model_calls.id."""
        orphans = exhaust_db.execute("""
            SELECT tc.id, tc.model_call_id FROM tool_calls tc
            WHERE tc.model_call_id IS NOT NULL
            AND tc.model_call_id NOT IN (SELECT id FROM model_calls)
        """).fetchall()
        assert len(orphans) == 0, (
            f"tool_calls with invalid model_call_id: {[dict(r) for r in orphans]}"
        )

    def test_tool_responses_call_fk(self, exhaust_db):
        """tool_responses.call_id references a valid tool_calls.id."""
        orphans = exhaust_db.execute("""
            SELECT tr.id, tr.call_id FROM tool_responses tr
            WHERE tr.call_id IS NOT NULL
            AND tr.call_id NOT IN (SELECT id FROM tool_calls)
        """).fetchall()
        assert len(orphans) == 0, (
            f"tool_responses with invalid call_id: {[dict(r) for r in orphans]}"
        )

    def test_mcp_origin_tool_calls_fk(self, exhaust_db):
        """tool_calls with origin='mcp' have a valid mcp_call_id FK."""
        orphans = exhaust_db.execute("""
            SELECT tc.id, tc.mcp_call_id FROM tool_calls tc
            WHERE tc.origin = 'mcp'
            AND tc.mcp_call_id IS NOT NULL
            AND tc.mcp_call_id NOT IN (SELECT id FROM mcp_calls)
        """).fetchall()
        assert len(orphans) == 0, (
            f"MCP tool_calls with invalid mcp_call_id: {[dict(r) for r in orphans]}"
        )

    def test_all_tables_have_id_column(self, exhaust_db):
        """All session.db tables have an 'id' primary key column."""
        tables = [
            r[0] for r in exhaust_db.execute(
                "SELECT name FROM sqlite_master WHERE type='table'"
            ).fetchall()
        ]
        for table in tables:
            if table == "sqlite_sequence":
                continue
            cols = [r[1] for r in exhaust_db.execute(f"PRAGMA table_info({table})").fetchall()]
            assert "id" in cols, f"Table {table} missing 'id' column"
