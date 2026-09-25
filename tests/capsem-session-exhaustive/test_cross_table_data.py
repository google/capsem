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
        """tool_responses.call_id references a valid tool_calls.call_id in the same trace."""
        orphans = exhaust_db.execute("""
            SELECT tr.id, tr.call_id, tr.trace_id FROM tool_responses tr
            LEFT JOIN tool_calls tc
              ON tr.call_id = tc.call_id
             AND tr.trace_id = tc.trace_id
            WHERE tc.call_id IS NULL
        """).fetchall()
        assert len(orphans) == 0, (
            f"tool_responses with invalid call_id: {[dict(r) for r in orphans]}"
        )

    def test_tool_responses_model_call_fk(self, exhaust_db):
        """tool_responses.model_call_id references the model exchange that consumed it."""
        orphans = exhaust_db.execute("""
            SELECT tr.id, tr.model_call_id, tr.call_id FROM tool_responses tr
            WHERE tr.model_call_id NOT IN (SELECT id FROM model_calls)
        """).fetchall()
        assert len(orphans) == 0, (
            f"tool_responses with invalid model_call_id: {[dict(r) for r in orphans]}"
        )

    def test_mcp_origin_tool_calls_fk(self, exhaust_db):
        """MCP-origin tool_calls are the protocol evidence; no side table exists."""
        rows = exhaust_db.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='mcp_calls'"
        ).fetchall()
        assert rows == []

    def test_all_tables_have_one_integer_primary_key(self, exhaust_db):
        """Every session.db row is addressed by one INTEGER PRIMARY KEY.

        Event tables call it `id`; a table keyed by what it stores names it
        for that (`body_blocks.block_offset`, `archive_state.singleton`).
        """
        tables = [
            r[0] for r in exhaust_db.execute(
                "SELECT name FROM sqlite_master WHERE type='table'"
            ).fetchall()
        ]
        for table in tables:
            if table == "sqlite_sequence":
                continue
            info = exhaust_db.execute(f"PRAGMA table_info({table})").fetchall()
            key = [(r[1], r[2].upper()) for r in info if r[5]]
            assert len(key) == 1 and key[0][1] == "INTEGER", (
                f"Table {table} is not keyed by one INTEGER PRIMARY KEY: {key}"
            )
