"""Error handling: operations on deleted/invalid VMs, concurrent VMs."""

import time
import uuid

import json

import pytest


def content_text(result):
    return result["content"][0]["text"]


def parse_content(result):
    return json.loads(result["content"][0]["text"])

pytestmark = pytest.mark.mcp


# ---------------------------------------------------------------------------
# Operations on deleted VMs
# ---------------------------------------------------------------------------


def test_exec_on_deleted_vm(mcp_session):
    vm_name = f"ex-{uuid.uuid4().hex[:4]}"
    mcp_session.call_tool("capsem_create", {"name": vm_name})
    mcp_session.call_tool("capsem_delete", {"id": vm_name})

    resp = mcp_session.call_tool_raw("capsem_exec", {
        "id": vm_name,
        "command": "echo should-fail",
    })
    result = resp.get("result", {})
    assert result.get("isError") is True or "error" in resp


def test_write_on_deleted_vm(mcp_session):
    vm_name = f"wr-{uuid.uuid4().hex[:4]}"
    mcp_session.call_tool("capsem_create", {"name": vm_name})
    mcp_session.call_tool("capsem_delete", {"id": vm_name})

    resp = mcp_session.call_tool_raw("capsem_write_file", {
        "id": vm_name,
        "path": "/tmp/x.txt",
        "content": "nope",
    })
    result = resp.get("result", {})
    assert result.get("isError") is True or "error" in resp


def test_read_on_deleted_vm(mcp_session):
    vm_name = f"rd-{uuid.uuid4().hex[:4]}"
    mcp_session.call_tool("capsem_create", {"name": vm_name})
    mcp_session.call_tool("capsem_delete", {"id": vm_name})

    resp = mcp_session.call_tool_raw("capsem_read_file", {
        "id": vm_name,
        "path": "/etc/os-release",
    })
    result = resp.get("result", {})
    assert result.get("isError") is True or "error" in resp


def test_info_on_deleted_vm(mcp_session):
    vm_name = f"in-{uuid.uuid4().hex[:4]}"
    mcp_session.call_tool("capsem_create", {"name": vm_name})
    mcp_session.call_tool("capsem_delete", {"id": vm_name})

    resp = mcp_session.call_tool_raw("capsem_info", {"id": vm_name})
    result = resp.get("result", {})
    assert result.get("isError") is True or "error" in resp


# ---------------------------------------------------------------------------
# Concurrent VMs (isolation)
# ---------------------------------------------------------------------------


def test_two_vms_isolated(mcp_session):
    """Two VMs with the same file path hold different contents."""
    vm_a = f"a-{uuid.uuid4().hex[:4]}"
    vm_b = f"b-{uuid.uuid4().hex[:4]}"

    res_a = mcp_session.call_tool("capsem_create", {"name": vm_a})
    assert not res_a.get("isError"), f"Failed to create VM A: {res_a}"
    
    res_b = mcp_session.call_tool("capsem_create", {"name": vm_b})
    assert not res_b.get("isError"), f"Failed to create VM B: {res_b}"

    try:
        # Wait for both to be exec-ready
        for vm in (vm_a, vm_b):
            ready = False
            for _ in range(60):
                try:
                    r = mcp_session.call_tool("capsem_exec", {
                        "id": vm,
                        "command": "echo ready",
                    })
                    if "ready" in content_text(r):
                        ready = True
                        break
                except (AssertionError, KeyError):
                    pass
                time.sleep(1)
            assert ready, f"VM {vm} never became exec-ready"

        # Write different data to same path
        mcp_session.call_tool("capsem_write_file", {
            "id": vm_a, "path": "/tmp/id.txt", "content": "vm-a",
        })
        mcp_session.call_tool("capsem_write_file", {
            "id": vm_b, "path": "/tmp/id.txt", "content": "vm-b",
        })

        # Verify isolation
        res_a = mcp_session.call_tool("capsem_read_file", {
            "id": vm_a, "path": "/tmp/id.txt",
        })
        res_b = mcp_session.call_tool("capsem_read_file", {
            "id": vm_b, "path": "/tmp/id.txt",
        })
        assert parse_content(res_a)["content"] == "vm-a"
        assert parse_content(res_b)["content"] == "vm-b"

        # Both present in list
        res = mcp_session.call_tool("capsem_list")
        ids = [s["id"] for s in parse_content(res)["sandboxes"]]
        assert vm_a in ids
        assert vm_b in ids
    finally:
        for vm in (vm_a, vm_b):
            try:
                mcp_session.call_tool("capsem_delete", {"id": vm})
            except Exception:
                pass
