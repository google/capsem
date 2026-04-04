"""VM lifecycle: create, list, info, delete and edge cases."""

import json
import time

import pytest


def parse_content(result):
    return json.loads(result["content"][0]["text"])

pytestmark = pytest.mark.mcp


def test_create_and_delete(mcp_session):
    vm_name = f"life-{int(time.time() * 1000)}"
    mcp_session.call_tool("capsem_create", {"name": vm_name})

    # Present in list
    res = mcp_session.call_tool("capsem_list")
    ids = [s["id"] for s in parse_content(res)["sandboxes"]]
    assert vm_name in ids

    # Delete
    mcp_session.call_tool("capsem_delete", {"id": vm_name})

    # Absent from list
    res = mcp_session.call_tool("capsem_list")
    ids = [s["id"] for s in parse_content(res)["sandboxes"]]
    assert vm_name not in ids


def test_create_with_resources(mcp_session):
    """Custom ramMb and cpuCount are accepted."""
    vm_name = f"res-{int(time.time() * 1000)}"
    mcp_session.call_tool("capsem_create", {
        "name": vm_name,
        "ramMb": 2048,
        "cpuCount": 2,
    })
    try:
        res = mcp_session.call_tool("capsem_info", {"id": vm_name})
        info = parse_content(res)
        assert info["id"] == vm_name
    finally:
        try:
            mcp_session.call_tool("capsem_delete", {"id": vm_name})
        except Exception:
            pass


def test_create_auto_name(mcp_session):
    """Create with no name -- service auto-generates an ID."""
    res = mcp_session.call_tool("capsem_create", {})
    data = parse_content(res)
    vm_id = data.get("id") or data.get("name")
    assert vm_id, f"No ID in create response: {data}"
    try:
        mcp_session.call_tool("capsem_delete", {"id": vm_id})
    except Exception:
        pass


def test_create_duplicate_name(mcp_session):
    """Second create with same name should error."""
    vm_name = f"dup-{int(time.time() * 1000)}"
    mcp_session.call_tool("capsem_create", {"name": vm_name})
    try:
        resp = mcp_session.call_tool_raw("capsem_create", {"name": vm_name})
        result = resp.get("result", {})
        assert result.get("isError") is True or "error" in resp
    finally:
        try:
            mcp_session.call_tool("capsem_delete", {"id": vm_name})
        except Exception:
            pass


def test_info_fields(shared_vm, mcp_session):
    """Info response must include id, status, pid."""
    vm_name, _ = shared_vm
    res = mcp_session.call_tool("capsem_info", {"id": vm_name})
    info = parse_content(res)
    assert info["id"] == vm_name
    assert "status" in info
    assert "pid" in info


def test_info_nonexistent(mcp_session):
    resp = mcp_session.call_tool_raw("capsem_info", {"id": "ghost-vm-404"})
    result = resp.get("result", {})
    assert result.get("isError") is True or "error" in resp


def test_delete_nonexistent(mcp_session):
    resp = mcp_session.call_tool_raw("capsem_delete", {"id": "no-such-vm-xyz"})
    result = resp.get("result", {})
    assert result.get("isError") is True or "error" in resp


import uuid

def test_delete_twice(mcp_session):
    """Deleting an already-deleted VM should error, not crash."""
    vm_name = f"d2x-{uuid.uuid4().hex[:4]}"
    mcp_session.call_tool("capsem_create", {"name": vm_name})
    mcp_session.call_tool("capsem_delete", {"id": vm_name})

    resp = mcp_session.call_tool_raw("capsem_delete", {"id": vm_name})
    result = resp.get("result", {})
    assert result.get("isError") is True or "error" in resp


def test_list_empty_start(mcp_session):
    """List should return a sandboxes array (may include shared_vm)."""
    res = mcp_session.call_tool("capsem_list")
    data = parse_content(res)
    assert "sandboxes" in data
    assert isinstance(data["sandboxes"], list)
