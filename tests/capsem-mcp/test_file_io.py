"""Guest file read/write operations."""

import json

import pytest


def parse_content(result):
    return json.loads(result["content"][0]["text"])

pytestmark = pytest.mark.mcp


def test_roundtrip(shared_vm, mcp_session):
    vm_name, _ = shared_vm
    mcp_session.call_tool("capsem_write_file", {
        "id": vm_name,
        "path": "/tmp/rt.txt",
        "content": "payload-abc",
    })
    res = mcp_session.call_tool("capsem_read_file", {
        "id": vm_name,
        "path": "/tmp/rt.txt",
    })
    assert parse_content(res)["content"] == "payload-abc"


def test_unicode(shared_vm, mcp_session):
    vm_name, _ = shared_vm
    text = "caf\u00e9 \u00fc\u00f1\u00ee\u00e7\u00f8\u00f0\u00e9"
    mcp_session.call_tool("capsem_write_file", {
        "id": vm_name,
        "path": "/tmp/uni.txt",
        "content": text,
    })
    res = mcp_session.call_tool("capsem_read_file", {
        "id": vm_name,
        "path": "/tmp/uni.txt",
    })
    assert parse_content(res)["content"] == text


def test_multiline(shared_vm, mcp_session):
    vm_name, _ = shared_vm
    text = "line1\nline2\nline3\n"
    mcp_session.call_tool("capsem_write_file", {
        "id": vm_name,
        "path": "/tmp/multi.txt",
        "content": text,
    })
    res = mcp_session.call_tool("capsem_read_file", {
        "id": vm_name,
        "path": "/tmp/multi.txt",
    })
    assert parse_content(res)["content"] == text


def test_empty_file(shared_vm, mcp_session):
    vm_name, _ = shared_vm
    mcp_session.call_tool("capsem_write_file", {
        "id": vm_name,
        "path": "/tmp/empty.txt",
        "content": "",
    })
    res = mcp_session.call_tool("capsem_read_file", {
        "id": vm_name,
        "path": "/tmp/empty.txt",
    })
    assert parse_content(res)["content"] == ""


def test_large_payload(shared_vm, mcp_session):
    """Write and read back ~100KB."""
    vm_name, _ = shared_vm
    text = "x" * 100_000
    mcp_session.call_tool("capsem_write_file", {
        "id": vm_name,
        "path": "/tmp/large.txt",
        "content": text,
    })
    res = mcp_session.call_tool("capsem_read_file", {
        "id": vm_name,
        "path": "/tmp/large.txt",
    })
    assert parse_content(res)["content"] == text


def test_overwrite(shared_vm, mcp_session):
    """Second write replaces file contents."""
    vm_name, _ = shared_vm
    mcp_session.call_tool("capsem_write_file", {
        "id": vm_name,
        "path": "/tmp/ow.txt",
        "content": "first",
    })
    mcp_session.call_tool("capsem_write_file", {
        "id": vm_name,
        "path": "/tmp/ow.txt",
        "content": "second",
    })
    res = mcp_session.call_tool("capsem_read_file", {
        "id": vm_name,
        "path": "/tmp/ow.txt",
    })
    assert parse_content(res)["content"] == "second"


def test_read_nonexistent(shared_vm, mcp_session):
    vm_name, _ = shared_vm
    resp = mcp_session.call_tool_raw("capsem_read_file", {
        "id": vm_name,
        "path": "/tmp/no-such-file-xyz.txt",
    })
    result = resp.get("result", {})
    assert result.get("isError") is True or "error" in resp


def test_write_nested_path(shared_vm, mcp_session):
    """Write to a nested directory (must exist or be auto-created)."""
    vm_name, _ = shared_vm
    # /tmp always exists, create a subdir via exec first
    mcp_session.call_tool("capsem_exec", {
        "id": vm_name,
        "command": "mkdir -p /tmp/nested/deep",
    })
    mcp_session.call_tool("capsem_write_file", {
        "id": vm_name,
        "path": "/tmp/nested/deep/file.txt",
        "content": "deep-payload",
    })
    res = mcp_session.call_tool("capsem_read_file", {
        "id": vm_name,
        "path": "/tmp/nested/deep/file.txt",
    })
    assert parse_content(res)["content"] == "deep-payload"
