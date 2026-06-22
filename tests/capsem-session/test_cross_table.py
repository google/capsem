"""Cross-table FK integrity in session.db."""

import pytest

pytestmark = pytest.mark.session


def test_tool_calls_reference_model_calls(session_db):
    """Model-attached tool_calls.model_call_id should reference model_calls.id."""
    orphans = session_db.execute("""
        SELECT tc.id, tc.model_call_id
        FROM tool_calls tc
        LEFT JOIN model_calls mc ON tc.model_call_id = mc.id
        WHERE tc.model_call_id IS NOT NULL AND mc.id IS NULL
    """).fetchall()
    assert len(orphans) == 0, f"Orphan tool_calls (no matching model_call): {orphans}"


def test_tool_responses_reference_valid_call_id(session_db):
    """tool_responses.call_id should reference a valid tool_calls.call_id."""
    orphans = session_db.execute("""
        SELECT tr.id, tr.call_id
        FROM tool_responses tr
        LEFT JOIN tool_calls tc ON tr.call_id = tc.call_id
        WHERE tc.call_id IS NULL
    """).fetchall()
    assert len(orphans) == 0, f"Orphan tool_responses (no matching tool_call): {orphans}"


def test_mcp_tool_calls_are_direct_tool_evidence(session_db):
    """MCP-origin tool invocations live in tool_calls, not mcp_calls."""
    orphans = session_db.execute("""
        SELECT tc.id, tc.mcp_call_id
        FROM tool_calls tc
        LEFT JOIN mcp_calls mc ON tc.mcp_call_id = mc.id
        WHERE tc.origin = 'mcp' AND tc.mcp_call_id IS NOT NULL AND mc.id IS NULL
    """).fetchall()
    assert len(orphans) == 0, f"MCP-origin tool_calls with invalid protocol link: {orphans}"


def test_snapshots_are_not_cross_table_activity(session_db):
    """Snapshots are exposed through VM snapshot routes, not session.db joins."""
    rows = session_db.execute(
        "SELECT name FROM sqlite_master WHERE type='table' AND name='snapshot_events'"
    ).fetchall()
    assert rows == []
