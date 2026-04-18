"""capsem_service_logs: read the last ~100KB of service.log with grep/tail."""

import pytest

from helpers.mcp import content_text

pytestmark = pytest.mark.mcp


def test_service_logs_present(mcp_session):
    """service_logs returns non-empty plain text (service.log is written by the fixture)."""
    # Trigger some recent activity so the log is populated.
    mcp_session.call_tool("capsem_list")

    text = content_text(mcp_session.call_tool("capsem_service_logs"))
    assert isinstance(text, str) and text, "service log empty"
    assert len(text) > 10, f"service log implausibly short: {text!r}"


def test_service_logs_tail(mcp_session):
    """tail=N limits the returned text to the last N lines."""
    mcp_session.call_tool("capsem_list")
    text = content_text(mcp_session.call_tool("capsem_service_logs", {"tail": 3}))
    assert isinstance(text, str) and text
    line_count = len(text.splitlines())
    assert line_count <= 3, f"tail=3 yielded {line_count} lines: {text!r}"


def test_service_logs_grep(mcp_session):
    """grep filters lines case-insensitively."""
    # Call a few tools so there's predictable log content to match against.
    mcp_session.call_tool("capsem_list")
    mcp_session.call_tool("capsem_version")

    text = content_text(mcp_session.call_tool("capsem_service_logs", {"grep": "GET"}))
    if not text:
        pytest.skip("service log did not contain any 'GET' lines to filter against")
    for line in text.splitlines():
        assert "get" in line.lower(), f"grep filter leaked line: {line!r}"
