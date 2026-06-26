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
    """tool_responses.call_id should reference a valid tool_calls.call_id in the same trace."""
    orphans = session_db.execute("""
        SELECT tr.id, tr.call_id, tr.trace_id
        FROM tool_responses tr
        LEFT JOIN tool_calls tc
          ON tr.call_id = tc.call_id
         AND tr.trace_id = tc.trace_id
        WHERE tc.call_id IS NULL
    """).fetchall()
    assert len(orphans) == 0, f"Orphan tool_responses (no matching tool_call): {orphans}"


def test_tool_responses_reference_model_calls(session_db):
    """tool_responses.model_call_id should reference the model exchange that consumed it."""
    orphans = session_db.execute("""
        SELECT tr.id, tr.model_call_id, tr.call_id
        FROM tool_responses tr
        LEFT JOIN model_calls mc ON tr.model_call_id = mc.id
        WHERE mc.id IS NULL
    """).fetchall()
    assert len(orphans) == 0, f"Orphan tool_responses (no matching model_call): {orphans}"


def test_mcp_tool_calls_are_direct_tool_evidence(session_db):
    """MCP-origin tool invocations live directly in tool_calls."""
    rows = session_db.execute(
        "SELECT name FROM sqlite_master WHERE type='table' AND name='mcp_calls'"
    ).fetchall()
    assert rows == []


def test_snapshots_are_not_cross_table_activity(session_db):
    """Snapshots are exposed through VM snapshot routes, not session.db joins."""
    rows = session_db.execute(
        "SELECT name FROM sqlite_master WHERE type='table' AND name='snapshot_events'"
    ).fetchall()
    assert rows == []
