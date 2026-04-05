"""Fork images: MCP tool tests for fork, image list/inspect/delete, create-from-image."""

import json
import time
import uuid

import pytest

pytestmark = pytest.mark.mcp


def parse_content(result):
    return json.loads(result["content"][0]["text"])


def content_text(result):
    return result["content"][0]["text"]


def _wait_exec_ready(session, vm_name, timeout=30):
    """Poll until a VM responds to exec via MCP."""
    for _ in range(timeout):
        try:
            res = session.call_tool("capsem_exec", {
                "id": vm_name,
                "command": "echo ready",
            })
            if "ready" in content_text(res):
                return True
        except (AssertionError, KeyError):
            pass
        time.sleep(1)
    return False


# ---------------------------------------------------------------------------
# Discovery
# ---------------------------------------------------------------------------

FORK_TOOLS = {
    "capsem_fork", "capsem_image_list",
    "capsem_image_inspect", "capsem_image_delete",
}


def test_fork_tools_discovered(mcp_session):
    """All 4 fork/image tools must appear in tools/list."""
    resp = mcp_session.request("tools/list")
    tools = {t["name"] for t in resp["result"]["tools"]}
    missing = FORK_TOOLS - tools
    assert not missing, f"Missing fork tools: {missing}"


def test_fork_schema_fields(mcp_session):
    """capsem_fork schema must declare id, name, description."""
    resp = mcp_session.request("tools/list")
    fork = next(t for t in resp["result"]["tools"] if t["name"] == "capsem_fork")
    props = fork["inputSchema"].get("properties", {})
    assert "id" in props, "Missing 'id' in fork schema"
    assert "name" in props, "Missing 'name' in fork schema"
    assert "description" in props, "Missing 'description' in fork schema"


def test_create_schema_has_image(mcp_session):
    """capsem_create schema must include the image parameter."""
    resp = mcp_session.request("tools/list")
    create = next(t for t in resp["result"]["tools"] if t["name"] == "capsem_create")
    props = create["inputSchema"].get("properties", {})
    assert "image" in props, "Missing 'image' in create schema"


# ---------------------------------------------------------------------------
# Full lifecycle
# ---------------------------------------------------------------------------


def test_full_lifecycle(mcp_session):
    """Create VM -> modify -> fork -> list -> inspect -> boot from image -> verify -> cleanup."""
    vm_name = f"fk-{uuid.uuid4().hex[:6]}"
    image_name = f"fi-{uuid.uuid4().hex[:6]}"
    forked_vm = f"ff-{uuid.uuid4().hex[:6]}"

    try:
        # 1. Create base VM
        mcp_session.call_tool("capsem_create", {"name": vm_name})
        assert _wait_exec_ready(mcp_session, vm_name), f"VM {vm_name} never exec-ready"

        # 2. Write marker file
        mcp_session.call_tool("capsem_exec", {
            "id": vm_name,
            "command": "echo 'mcp-fork-marker' > /root/fork_test.txt",
        })
        res = mcp_session.call_tool("capsem_exec", {
            "id": vm_name,
            "command": "cat /root/fork_test.txt",
        })
        assert "mcp-fork-marker" in content_text(res)

        # 3. Fork
        res = mcp_session.call_tool("capsem_fork", {
            "id": vm_name,
            "name": image_name,
            "description": "MCP lifecycle test image",
        })
        fork_data = parse_content(res)
        assert fork_data["name"] == image_name

        # 4. List images
        res = mcp_session.call_tool("capsem_image_list")
        list_data = parse_content(res)
        names = [img["name"] for img in list_data["images"]]
        assert image_name in names

        # 5. Inspect image
        res = mcp_session.call_tool("capsem_image_inspect", {"name": image_name})
        info = parse_content(res)
        assert info["name"] == image_name
        assert info["description"] == "MCP lifecycle test image"
        assert info["source_vm"] == vm_name

        # 6. Boot from image
        mcp_session.call_tool("capsem_create", {
            "name": forked_vm,
            "image": image_name,
        })
        assert _wait_exec_ready(mcp_session, forked_vm), f"Forked VM {forked_vm} never exec-ready"

        # 7. Verify marker persisted
        res = mcp_session.call_tool("capsem_exec", {
            "id": forked_vm,
            "command": "cat /root/fork_test.txt",
        })
        assert "mcp-fork-marker" in content_text(res)
    finally:
        for vm in [forked_vm, vm_name]:
            try:
                mcp_session.call_tool("capsem_delete", {"id": vm})
            except Exception:
                pass
        try:
            mcp_session.call_tool("capsem_image_delete", {"name": image_name})
        except Exception:
            pass


def test_fork_with_file_io(mcp_session):
    """Fork lifecycle using write_file/read_file instead of exec."""
    vm_name = f"fw-{uuid.uuid4().hex[:6]}"
    image_name = f"wi-{uuid.uuid4().hex[:6]}"
    forked_vm = f"wf-{uuid.uuid4().hex[:6]}"

    try:
        # 1. Create base VM, wait for exec-ready
        mcp_session.call_tool("capsem_create", {"name": vm_name})
        assert _wait_exec_ready(mcp_session, vm_name), f"VM {vm_name} never exec-ready"

        # 2. Write file via write_file tool
        mcp_session.call_tool("capsem_write_file", {
            "id": vm_name,
            "path": "/root/io_test.txt",
            "content": "file-io-marker",
        })

        # 3. Verify with read_file
        res = mcp_session.call_tool("capsem_read_file", {
            "id": vm_name,
            "path": "/root/io_test.txt",
        })
        assert "file-io-marker" in content_text(res)

        # 4. Fork
        mcp_session.call_tool("capsem_fork", {
            "id": vm_name,
            "name": image_name,
        })

        # 5. Boot from image
        mcp_session.call_tool("capsem_create", {
            "name": forked_vm,
            "image": image_name,
        })
        assert _wait_exec_ready(mcp_session, forked_vm), f"Forked VM {forked_vm} never exec-ready"

        # 6. Verify file persisted via read_file
        res = mcp_session.call_tool("capsem_read_file", {
            "id": forked_vm,
            "path": "/root/io_test.txt",
        })
        assert "file-io-marker" in content_text(res)

        # 7. Also verify via exec (belt and suspenders)
        res = mcp_session.call_tool("capsem_exec", {
            "id": forked_vm,
            "command": "cat /root/io_test.txt",
        })
        assert "file-io-marker" in content_text(res)
    finally:
        for vm in [forked_vm, vm_name]:
            try:
                mcp_session.call_tool("capsem_delete", {"id": vm})
            except Exception:
                pass
        try:
            mcp_session.call_tool("capsem_image_delete", {"name": image_name})
        except Exception:
            pass


def test_fork_of_fork(mcp_session):
    """Fork a VM, boot from image, modify, fork again -- second image has both layers."""
    vm1 = f"f1-{uuid.uuid4().hex[:6]}"
    img1 = f"i1-{uuid.uuid4().hex[:6]}"
    vm2 = f"f2-{uuid.uuid4().hex[:6]}"
    img2 = f"i2-{uuid.uuid4().hex[:6]}"
    vm3 = f"f3-{uuid.uuid4().hex[:6]}"

    try:
        # Layer 1: create VM, write file, fork
        mcp_session.call_tool("capsem_create", {"name": vm1})
        assert _wait_exec_ready(mcp_session, vm1), f"VM {vm1} never exec-ready"
        mcp_session.call_tool("capsem_write_file", {
            "id": vm1, "path": "/root/layer1.txt", "content": "base-layer",
        })
        mcp_session.call_tool("capsem_fork", {"id": vm1, "name": img1})

        # Layer 2: boot from img1, add second file, fork again
        mcp_session.call_tool("capsem_create", {"name": vm2, "image": img1})
        assert _wait_exec_ready(mcp_session, vm2), f"VM {vm2} never exec-ready"

        # Verify layer 1 carried over
        res = mcp_session.call_tool("capsem_read_file", {
            "id": vm2, "path": "/root/layer1.txt",
        })
        assert "base-layer" in content_text(res)

        # Add layer 2
        mcp_session.call_tool("capsem_write_file", {
            "id": vm2, "path": "/root/layer2.txt", "content": "second-layer",
        })
        mcp_session.call_tool("capsem_fork", {"id": vm2, "name": img2})

        # Boot from img2 -- should have both files
        mcp_session.call_tool("capsem_create", {"name": vm3, "image": img2})
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
        for vm in [vm3, vm2, vm1]:
            try:
                mcp_session.call_tool("capsem_delete", {"id": vm})
            except Exception:
                pass
        for img in [img2, img1]:
            try:
                mcp_session.call_tool("capsem_image_delete", {"name": img})
            except Exception:
                pass


def test_delete_parent_image_forked_still_boots(mcp_session):
    """Delete the parent image -- a VM already booted from it keeps working, and a second image forked from that VM still boots."""
    vm1 = f"dp-{uuid.uuid4().hex[:6]}"
    img1 = f"pi-{uuid.uuid4().hex[:6]}"
    vm2 = f"dv-{uuid.uuid4().hex[:6]}"

    try:
        # Create base VM, write marker, fork to img1
        mcp_session.call_tool("capsem_create", {"name": vm1})
        assert _wait_exec_ready(mcp_session, vm1), f"VM {vm1} never exec-ready"
        mcp_session.call_tool("capsem_write_file", {
            "id": vm1, "path": "/root/parent.txt", "content": "from-parent",
        })
        mcp_session.call_tool("capsem_fork", {"id": vm1, "name": img1})

        # Boot vm2 from img1
        mcp_session.call_tool("capsem_create", {"name": vm2, "image": img1})
        assert _wait_exec_ready(mcp_session, vm2), f"VM {vm2} never exec-ready"

        # Delete the parent image while vm2 is running
        mcp_session.call_tool("capsem_image_delete", {"name": img1})

        # vm2 should still work -- it has its own copy of the data
        res = mcp_session.call_tool("capsem_read_file", {
            "id": vm2, "path": "/root/parent.txt",
        })
        assert "from-parent" in content_text(res)

        # Also verify via exec
        res = mcp_session.call_tool("capsem_exec", {
            "id": vm2, "command": "cat /root/parent.txt",
        })
        assert "from-parent" in content_text(res)

        # Image should be gone
        res = mcp_session.call_tool("capsem_image_list")
        list_data = parse_content(res)
        names = [img["name"] for img in list_data["images"]]
        assert img1 not in names
    finally:
        for vm in [vm2, vm1]:
            try:
                mcp_session.call_tool("capsem_delete", {"id": vm})
            except Exception:
                pass
        try:
            mcp_session.call_tool("capsem_image_delete", {"name": img1})
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


def test_fork_duplicate_image_name(mcp_session):
    """Forking to an already-existing image name should fail."""
    vm_name = f"ds-{uuid.uuid4().hex[:6]}"
    image_name = f"di-{uuid.uuid4().hex[:6]}"

    try:
        mcp_session.call_tool("capsem_create", {"name": vm_name})
        assert _wait_exec_ready(mcp_session, vm_name), f"VM {vm_name} never exec-ready"

        # First fork succeeds
        mcp_session.call_tool("capsem_fork", {
            "id": vm_name,
            "name": image_name,
        })

        # Second fork to same name should fail
        resp = mcp_session.call_tool_raw("capsem_fork", {
            "id": vm_name,
            "name": image_name,
        })
        result = resp.get("result", {})
        assert result.get("isError") is True or "error" in resp
    finally:
        try:
            mcp_session.call_tool("capsem_delete", {"id": vm_name})
        except Exception:
            pass
        try:
            mcp_session.call_tool("capsem_image_delete", {"name": image_name})
        except Exception:
            pass


def test_inspect_nonexistent_image(mcp_session):
    """Inspecting a non-existent image should return an error."""
    resp = mcp_session.call_tool_raw("capsem_image_inspect", {
        "name": "no-such-image-999",
    })
    result = resp.get("result", {})
    assert result.get("isError") is True or "error" in resp


def test_delete_nonexistent_image(mcp_session):
    """Deleting a non-existent image should return an error."""
    resp = mcp_session.call_tool_raw("capsem_image_delete", {
        "name": "no-such-image-999",
    })
    result = resp.get("result", {})
    assert result.get("isError") is True or "error" in resp


def test_create_from_nonexistent_image(mcp_session):
    """Creating a VM from a non-existent image should fail."""
    resp = mcp_session.call_tool_raw("capsem_create", {
        "name": f"vm-{uuid.uuid4().hex[:8]}",
        "image": "no-such-image-999",
    })
    result = resp.get("result", {})
    assert result.get("isError") is True or "error" in resp


def test_image_list_returns_valid_response(mcp_session):
    """capsem_image_list should return a response with images array."""
    res = mcp_session.call_tool("capsem_image_list")
    data = parse_content(res)
    assert "images" in data
    assert isinstance(data["images"], list)
