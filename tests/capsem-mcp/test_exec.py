"""Command execution inside guest VMs."""

import json

import pytest


def content_text(result):
    return result["content"][0]["text"]

pytestmark = pytest.mark.mcp


def test_stdout(shared_vm, mcp_session):
    vm_name, _ = shared_vm
    res = mcp_session.call_tool("capsem_exec", {
        "id": vm_name,
        "command": "echo hello-mcp",
    })
    assert "hello-mcp" in content_text(res)


def test_stderr(shared_vm, mcp_session):
    vm_name, _ = shared_vm
    res = mcp_session.call_tool("capsem_exec", {
        "id": vm_name,
        "command": "echo err-msg >&2",
    })
    assert "err-msg" in content_text(res)


def test_exit_code_nonzero(shared_vm, mcp_session):
    """A failing command should surface the nonzero exit code."""
    vm_name, _ = shared_vm
    resp = mcp_session.call_tool_raw("capsem_exec", {
        "id": vm_name,
        "command": "exit 42",
    })
    result = resp.get("result", {})
    text = content_text(result) if result.get("content") else ""
    has_error = result.get("isError") is True
    has_exit_code = "42" in text
    assert has_error or has_exit_code, f"Expected nonzero exit info: {resp}"


def test_multiline_output(shared_vm, mcp_session):
    vm_name, _ = shared_vm
    res = mcp_session.call_tool("capsem_exec", {
        "id": vm_name,
        "command": "printf 'line1\\nline2\\nline3'",
    })
    text = content_text(res)
    assert "line1" in text
    assert "line2" in text
    assert "line3" in text


def test_pipe(shared_vm, mcp_session):
    vm_name, _ = shared_vm
    res = mcp_session.call_tool("capsem_exec", {
        "id": vm_name,
        "command": "echo abc123 | grep -o abc",
    })
    assert "abc" in content_text(res)


def test_env_var(shared_vm, mcp_session):
    vm_name, _ = shared_vm
    res = mcp_session.call_tool("capsem_exec", {
        "id": vm_name,
        "command": "export X=hello && echo $X",
    })
    assert "hello" in content_text(res)


def test_special_chars(shared_vm, mcp_session):
    """Quotes, ampersands, and other shell metacharacters pass through."""
    vm_name, _ = shared_vm
    res = mcp_session.call_tool("capsem_exec", {
        "id": vm_name,
        "command": "echo 'hello world & \"quotes\"'",
    })
    text = content_text(res)
    assert "hello world" in text
    assert "quotes" in text


def test_which_shows_path(shared_vm, mcp_session):
    """Basic coreutils are available in the guest."""
    vm_name, _ = shared_vm
    res = mcp_session.call_tool("capsem_exec", {
        "id": vm_name,
        "command": "which ls",
    })
    assert "/bin/ls" in content_text(res) or "/usr/bin/ls" in content_text(res)


def test_uname(shared_vm, mcp_session):
    """Guest runs Linux."""
    vm_name, _ = shared_vm
    res = mcp_session.call_tool("capsem_exec", {
        "id": vm_name,
        "command": "uname -s",
    })
    assert "Linux" in content_text(res)
