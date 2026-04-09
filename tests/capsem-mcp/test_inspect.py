"""Telemetry DB inspection tools."""

import pytest

from helpers.mcp import content_text

pytestmark = pytest.mark.mcp


def test_schema(mcp_session):
    """inspect_schema returns SQL CREATE TABLE statements."""
    res = mcp_session.call_tool("capsem_inspect_schema")
    assert "CREATE TABLE" in content_text(res)


def test_sql_query(shared_vm, mcp_session):
    """Run a simple SELECT against the telemetry DB."""
    vm_name, _ = shared_vm
    res = mcp_session.call_tool("capsem_inspect", {
        "id": vm_name,
        "sql": "SELECT name FROM sqlite_master WHERE type='table'",
    })
    text = content_text(res)
    assert len(text) > 0


def test_bad_sql(shared_vm, mcp_session):
    """Invalid SQL should return an error, not crash."""
    vm_name, _ = shared_vm
    resp = mcp_session.call_tool_raw("capsem_inspect", {
        "id": vm_name,
        "sql": "THIS IS NOT SQL",
    })
    result = resp.get("result", {})
    assert result.get("isError") is True or "error" in resp


def test_inspect_nonexistent_vm(mcp_session):
    resp = mcp_session.call_tool_raw("capsem_inspect", {
        "id": "ghost-vm-inspect",
        "sql": "SELECT 1",
    })
    result = resp.get("result", {})
    assert result.get("isError") is True or "error" in resp
