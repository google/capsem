"""Verify model_calls are logged when LLM API calls occur."""

import pytest

pytestmark = pytest.mark.session


def test_model_calls_table_exists(session_db):
    tables = [r[0] for r in session_db.execute(
        "SELECT name FROM sqlite_master WHERE type='table'"
    ).fetchall()]
    assert "model_calls" in tables


def test_model_calls_schema(session_db):
    cols = [r[1] for r in session_db.execute("PRAGMA table_info(model_calls)").fetchall()]
    for required in ["provider", "model", "input_tokens", "output_tokens",
                     "estimated_cost_usd", "trace_id", "duration_ms"]:
        assert required in cols, f"Missing column: {required}"


def test_tool_calls_table_exists(session_db):
    tables = [r[0] for r in session_db.execute(
        "SELECT name FROM sqlite_master WHERE type='table'"
    ).fetchall()]
    assert "tool_calls" in tables


def test_tool_calls_schema(session_db):
    cols = [r[1] for r in session_db.execute("PRAGMA table_info(tool_calls)").fetchall()]
    for required in ["model_call_id", "tool_name", "origin", "arguments"]:
        assert required in cols, f"Missing column: {required}"


def test_tool_responses_table_exists(session_db):
    tables = [r[0] for r in session_db.execute(
        "SELECT name FROM sqlite_master WHERE type='table'"
    ).fetchall()]
    assert "tool_responses" in tables


def test_tool_responses_schema(session_db):
    cols = [r[1] for r in session_db.execute("PRAGMA table_info(tool_responses)").fetchall()]
    for required in ["model_call_id", "call_id", "content_preview", "is_error"]:
        assert required in cols, f"Missing column: {required}"
