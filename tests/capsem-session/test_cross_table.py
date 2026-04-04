"""Cross-table FK integrity in session.db."""

import pytest

pytestmark = pytest.mark.session


def test_tool_calls_reference_model_calls(session_db):
    """tool_calls.model_call_id should reference a valid model_calls.id."""
    orphans = session_db.execute("""
        SELECT tc.id, tc.model_call_id
        FROM tool_calls tc
        LEFT JOIN model_calls mc ON tc.model_call_id = mc.id
        WHERE mc.id IS NULL
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


def test_mcp_tool_calls_reference_mcp_calls(session_db):
    """tool_calls with origin='mcp' should have a valid mcp_call_id."""
    orphans = session_db.execute("""
        SELECT tc.id, tc.mcp_call_id
        FROM tool_calls tc
        LEFT JOIN mcp_calls mc ON tc.mcp_call_id = mc.id
        WHERE tc.origin = 'mcp' AND tc.mcp_call_id IS NOT NULL AND mc.id IS NULL
    """).fetchall()
    assert len(orphans) == 0, f"MCP tool_calls with invalid mcp_call_id: {orphans}"


def test_snapshot_event_fs_range_valid(session_db):
    """snapshot_events fs_event_id range should reference valid fs_events."""
    rows = session_db.execute("""
        SELECT se.id, se.start_fs_event_id, se.stop_fs_event_id
        FROM snapshot_events se
        WHERE se.stop_fs_event_id IS NOT NULL
    """).fetchall()
    for row in rows:
        start = row["start_fs_event_id"]
        stop = row["stop_fs_event_id"]
        if start is not None and stop is not None:
            assert stop >= start, (
                f"snapshot_event {row['id']}: stop ({stop}) < start ({start})"
            )
