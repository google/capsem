"""MCP fork lifecycle tests.

The service no longer has a separate "image" concept -- a forked VM is
just another persistent VM. These tests exercise capsem_fork end to end
(create -> write -> fork -> boot-from-fork -> verify), plus the error
paths that used to live under capsem_image_*.
"""

import uuid

import pytest

from helpers.mcp import content_text, parse_content, wait_exec_ready as _wait_exec_ready

pytestmark = pytest.mark.mcp


# ---------------------------------------------------------------------------
# Discovery
# ---------------------------------------------------------------------------


def test_fork_tool_discovered(mcp_session):
    """capsem_fork must appear in tools/list."""
    resp = mcp_session.request("tools/list")
    tools = {t["name"] for t in resp["result"]["tools"]}
    assert "capsem_fork" in tools, "Missing capsem_fork"


def test_fork_schema_fields(mcp_session):
    """capsem_fork schema must declare id, name, description."""
    resp = mcp_session.request("tools/list")
    fork = next(t for t in resp["result"]["tools"] if t["name"] == "capsem_fork")
    props = fork["inputSchema"].get("properties", {})
    assert "id" in props, "Missing 'id' in fork schema"
    assert "name" in props, "Missing 'name' in fork schema"
    assert "description" in props, "Missing 'description' in fork schema"


def test_create_schema_has_from(mcp_session):
    """capsem_create schema must include the `from` parameter (fork source)."""
    resp = mcp_session.request("tools/list")
    create = next(t for t in resp["result"]["tools"] if t["name"] == "capsem_create")
    props = create["inputSchema"].get("properties", {})
    assert "from" in props, "Missing 'from' in create schema"


# ---------------------------------------------------------------------------
# Full lifecycle
# ---------------------------------------------------------------------------


def test_full_lifecycle(mcp_session):
    """Create VM -> modify -> fork -> list -> info -> boot from fork -> verify -> cleanup."""
    vm_name = f"fk-{uuid.uuid4().hex[:6]}"
    fork_name = f"fi-{uuid.uuid4().hex[:6]}"
    child_vm = f"ff-{uuid.uuid4().hex[:6]}"

    try:
        mcp_session.call_tool("capsem_create", {"name": vm_name})
        assert _wait_exec_ready(mcp_session, vm_name), f"VM {vm_name} never exec-ready"

        mcp_session.call_tool("capsem_exec", {
            "id": vm_name,
            "command": "echo 'mcp-fork-marker' > /root/fork_test.txt",
        })
        res = mcp_session.call_tool("capsem_exec", {
            "id": vm_name,
            "command": "cat /root/fork_test.txt",
        })
        assert "mcp-fork-marker" in content_text(res)

        res = mcp_session.call_tool("capsem_fork", {
            "id": vm_name,
            "name": fork_name,
            "description": "MCP lifecycle test fork",
        })
        fork_data = parse_content(res)
        assert fork_data["name"] == fork_name

        # Fork appears in capsem_list as a stopped persistent VM
        res = mcp_session.call_tool("capsem_list")
        list_data = parse_content(res)
        names = [s.get("name") for s in list_data.get("sandboxes", [])]
        assert fork_name in names

        # Fork's metadata via capsem_info
        res = mcp_session.call_tool("capsem_info", {"id": fork_name})
        info = parse_content(res)
        assert info["name"] == fork_name
        assert info["description"] == "MCP lifecycle test fork"
        assert info["forked_from"] == vm_name

        # Boot a child from the fork
        mcp_session.call_tool("capsem_create", {
            "name": child_vm,
            "from": fork_name,
        })
        assert _wait_exec_ready(mcp_session, child_vm), f"Child VM {child_vm} never exec-ready"

        res = mcp_session.call_tool("capsem_exec", {
            "id": child_vm,
            "command": "cat /root/fork_test.txt",
        })
        assert "mcp-fork-marker" in content_text(res)
    finally:
        for vm in [child_vm, fork_name, vm_name]:
            try:
                mcp_session.call_tool("capsem_delete", {"id": vm})
            except Exception:
                pass


def test_fork_with_file_io(mcp_session):
    """Fork lifecycle using write_file/read_file instead of exec."""
    vm_name = f"fw-{uuid.uuid4().hex[:6]}"
    fork_name = f"wi-{uuid.uuid4().hex[:6]}"
    child_vm = f"wf-{uuid.uuid4().hex[:6]}"

    try:
        mcp_session.call_tool("capsem_create", {"name": vm_name})
        assert _wait_exec_ready(mcp_session, vm_name), f"VM {vm_name} never exec-ready"

        mcp_session.call_tool("capsem_write_file", {
            "id": vm_name,
            "path": "/root/io_test.txt",
            "content": "file-io-marker",
        })
        res = mcp_session.call_tool("capsem_read_file", {
            "id": vm_name,
            "path": "/root/io_test.txt",
        })
        assert "file-io-marker" in content_text(res)

        mcp_session.call_tool("capsem_fork", {
            "id": vm_name,
            "name": fork_name,
        })

        mcp_session.call_tool("capsem_create", {
            "name": child_vm,
            "from": fork_name,
        })
        assert _wait_exec_ready(mcp_session, child_vm), f"Child VM {child_vm} never exec-ready"

        res = mcp_session.call_tool("capsem_read_file", {
            "id": child_vm,
            "path": "/root/io_test.txt",
        })
        assert "file-io-marker" in content_text(res)

        res = mcp_session.call_tool("capsem_exec", {
            "id": child_vm,
            "command": "cat /root/io_test.txt",
        })
        assert "file-io-marker" in content_text(res)
    finally:
        for vm in [child_vm, fork_name, vm_name]:
            try:
                mcp_session.call_tool("capsem_delete", {"id": vm})
            except Exception:
                pass


def test_fork_of_fork(mcp_session):
    """Fork a VM, boot from fork, modify, fork again -- second fork has both layers."""
    vm1 = f"f1-{uuid.uuid4().hex[:6]}"
    fork1 = f"i1-{uuid.uuid4().hex[:6]}"
    vm2 = f"f2-{uuid.uuid4().hex[:6]}"
    fork2 = f"i2-{uuid.uuid4().hex[:6]}"
    vm3 = f"f3-{uuid.uuid4().hex[:6]}"

    try:
        mcp_session.call_tool("capsem_create", {"name": vm1})
        assert _wait_exec_ready(mcp_session, vm1), f"VM {vm1} never exec-ready"
        mcp_session.call_tool("capsem_write_file", {
            "id": vm1, "path": "/root/layer1.txt", "content": "base-layer",
        })
        mcp_session.call_tool("capsem_fork", {"id": vm1, "name": fork1})

        mcp_session.call_tool("capsem_create", {"name": vm2, "from": fork1})
        assert _wait_exec_ready(mcp_session, vm2), f"VM {vm2} never exec-ready"

        res = mcp_session.call_tool("capsem_read_file", {
            "id": vm2, "path": "/root/layer1.txt",
        })
        assert "base-layer" in content_text(res)

        mcp_session.call_tool("capsem_write_file", {
            "id": vm2, "path": "/root/layer2.txt", "content": "second-layer",
        })
        mcp_session.call_tool("capsem_fork", {"id": vm2, "name": fork2})

        mcp_session.call_tool("capsem_create", {"name": vm3, "from": fork2})
        assert _wait_exec_ready(mcp_session, vm3), f"VM {vm3} never exec-ready"

        res = mcp_session.call_tool("capsem_read_file", {
            "id": vm3, "path": "/root/layer1.txt",
        })
        assert "base-layer" in content_text(res)

        res = mcp_session.call_tool("capsem_read_file", {
            "id": vm3, "path": "/root/layer2.txt",
        })
        assert "second-layer" in content_text(res)
    finally:
        for vm in [vm3, vm2, vm1, fork2, fork1]:
            try:
                mcp_session.call_tool("capsem_delete", {"id": vm})
            except Exception:
                pass


def test_delete_parent_fork_child_still_boots(mcp_session):
    """Delete the parent fork -- a child VM already booted from it keeps working."""
    vm1 = f"dp-{uuid.uuid4().hex[:6]}"
    fork1 = f"pi-{uuid.uuid4().hex[:6]}"
    vm2 = f"dv-{uuid.uuid4().hex[:6]}"

    try:
        mcp_session.call_tool("capsem_create", {"name": vm1})
        assert _wait_exec_ready(mcp_session, vm1), f"VM {vm1} never exec-ready"
        mcp_session.call_tool("capsem_write_file", {
            "id": vm1, "path": "/root/parent.txt", "content": "from-parent",
        })
        mcp_session.call_tool("capsem_fork", {"id": vm1, "name": fork1})

        mcp_session.call_tool("capsem_create", {"name": vm2, "from": fork1})
        assert _wait_exec_ready(mcp_session, vm2), f"VM {vm2} never exec-ready"

        # Delete the parent fork while vm2 is running
        mcp_session.call_tool("capsem_delete", {"id": fork1})

        # vm2 still has its own copy of the data
        res = mcp_session.call_tool("capsem_read_file", {
            "id": vm2, "path": "/root/parent.txt",
        })
        assert "from-parent" in content_text(res)
        res = mcp_session.call_tool("capsem_exec", {
            "id": vm2, "command": "cat /root/parent.txt",
        })
        assert "from-parent" in content_text(res)

        # Parent fork is gone from capsem_list
        res = mcp_session.call_tool("capsem_list")
        list_data = parse_content(res)
        names = [s.get("name") for s in list_data.get("sandboxes", [])]
        assert fork1 not in names
    finally:
        for vm in [vm2, vm1, fork1]:
            try:
                mcp_session.call_tool("capsem_delete", {"id": vm})
            except Exception:
                pass


# ---------------------------------------------------------------------------
# Error cases
# ---------------------------------------------------------------------------


def test_fork_nonexistent_vm(mcp_session):
    """Forking a non-existent VM should return an error."""
    resp = mcp_session.call_tool_raw("capsem_fork", {
        "id": "ghost-vm-999",
        "name": f"img-{uuid.uuid4().hex[:8]}",
    })
    result = resp.get("result", {})
    assert result.get("isError") is True or "error" in resp


def test_fork_duplicate_name(mcp_session):
    """Forking to an already-existing name should fail."""
    vm_name = f"ds-{uuid.uuid4().hex[:6]}"
    fork_name = f"di-{uuid.uuid4().hex[:6]}"

    try:
        mcp_session.call_tool("capsem_create", {"name": vm_name})
        assert _wait_exec_ready(mcp_session, vm_name), f"VM {vm_name} never exec-ready"

        mcp_session.call_tool("capsem_fork", {"id": vm_name, "name": fork_name})

        resp = mcp_session.call_tool_raw("capsem_fork", {
            "id": vm_name,
            "name": fork_name,
        })
        result = resp.get("result", {})
        assert result.get("isError") is True or "error" in resp
    finally:
        for vm in [vm_name, fork_name]:
            try:
                mcp_session.call_tool("capsem_delete", {"id": vm})
            except Exception:
                pass


def test_create_from_nonexistent_fork(mcp_session):
    """Creating a VM from a non-existent source should fail."""
    resp = mcp_session.call_tool_raw("capsem_create", {
        "name": f"vm-{uuid.uuid4().hex[:8]}",
        "from": "no-such-source-999",
    })
    result = resp.get("result", {})
    assert result.get("isError") is True or "error" in resp
