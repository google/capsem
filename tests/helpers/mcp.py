"""Shared helpers for capsem-mcp integration tests.

Centralises parse_content, content_text, and readiness-polling functions
that were previously duplicated across every MCP test module.
"""

import json

from .constants import EXEC_READY_TIMEOUT


def parse_content(result):
    """Extract and JSON-parse the first content text from a tool result."""
    return json.loads(result["content"][0]["text"])


def content_text(result):
    """Extract the raw text from the first content block."""
    return result["content"][0]["text"]


def wait_exec_ready(session, vm_name, timeout=EXEC_READY_TIMEOUT):
    """Wait until a VM responds to exec via MCP.

    The server polls internally for VM readiness, so a single call with
    adequate timeout is sufficient.
    """
    try:
        res = session.call_tool("capsem_exec", {
            "id": vm_name,
            "command": "echo ready",
            "timeout_secs": timeout,
        })
        return "ready" in content_text(res)
    except (AssertionError, KeyError):
        return False


def wait_file_ready(session, vm_name, timeout=EXEC_READY_TIMEOUT):
    """Wait until a VM responds to write_file+read_file roundtrip.

    The server polls internally for VM readiness, so a single call with
    adequate timeout is sufficient.
    """
    probe_path = "/root/.capsem-ready-probe"
    try:
        session.call_tool("capsem_write_file", {
            "id": vm_name, "path": probe_path, "content": "ready",
        })
        res = session.call_tool("capsem_read_file", {
            "id": vm_name, "path": probe_path,
        })
        data = parse_content(res)
        return data.get("content") == "ready"
    except (AssertionError, KeyError, json.JSONDecodeError):
        return False
