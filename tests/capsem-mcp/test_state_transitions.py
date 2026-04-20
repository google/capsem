"""State-changing MCP tools: suspend, persist, purge."""

import uuid

import pytest

from helpers.constants import EXEC_READY_TIMEOUT
from helpers.mcp import parse_content, wait_exec_ready

pytestmark = pytest.mark.mcp


def test_suspend_and_resume_persistent(fresh_vm, mcp_session):
    """Suspend flips status Running -> Suspended; resume restores it and state survives."""
    vm_name = fresh_vm()
    assert wait_exec_ready(mcp_session, vm_name, timeout=EXEC_READY_TIMEOUT), (
        f"{vm_name} never exec-ready"
    )

    # Write a marker file so we can verify suspend preserves state.
    mcp_session.call_tool("capsem_write_file", {
        "id": vm_name,
        "path": "/root/marker.txt",
        "content": "persisted-through-suspend",
    })

    mcp_session.call_tool("capsem_suspend", {"id": vm_name})

    info = parse_content(mcp_session.call_tool("capsem_info", {"id": vm_name}))
    assert info["status"] == "Suspended", f"status after suspend: {info['status']!r}"

    mcp_session.call_tool("capsem_resume", {"name": vm_name})
    assert wait_exec_ready(mcp_session, vm_name, timeout=EXEC_READY_TIMEOUT), (
        "VM did not become exec-ready after resume"
    )

    info = parse_content(mcp_session.call_tool("capsem_info", {"id": vm_name}))
    assert info["status"] == "Running", f"status after resume: {info['status']!r}"

    res = mcp_session.call_tool("capsem_read_file", {
        "id": vm_name,
        "path": "/root/marker.txt",
    })
    assert parse_content(res)["content"] == "persisted-through-suspend"


def test_suspend_ephemeral_rejected(mcp_session):
    """capsem_suspend must reject ephemeral (non-persistent) sessions."""
    data = parse_content(mcp_session.call_tool("capsem_create", {}))
    vm_id = data.get("id") or data.get("name")
    assert vm_id, f"no id in create response: {data}"
    try:
        assert wait_exec_ready(mcp_session, vm_id, timeout=EXEC_READY_TIMEOUT), (
            f"ephemeral {vm_id} never exec-ready"
        )
        resp = mcp_session.call_tool_raw("capsem_suspend", {"id": vm_id})
        result = resp.get("result", {})
        assert result.get("isError") is True or "error" in resp, (
            f"expected error suspending ephemeral, got: {resp}"
        )
    finally:
        try:
            mcp_session.call_tool("capsem_delete", {"id": vm_id})
        except Exception:
            pass


def test_persist_converts_ephemeral(mcp_session):
    """capsem_persist converts a running ephemeral session to persistent."""
    data = parse_content(mcp_session.call_tool("capsem_create", {}))
    vm_id = data.get("id") or data.get("name")
    assert vm_id, f"no id in create response: {data}"

    new_name = f"persisted-{uuid.uuid4().hex[:8]}"
    try:
        mcp_session.call_tool("capsem_persist", {"id": vm_id, "name": new_name})

        # After persist the sandbox is known by its new name.
        listing = parse_content(mcp_session.call_tool("capsem_list"))
        ids = {s["id"] for s in listing["sandboxes"]}
        assert new_name in ids, f"{new_name} missing from list after persist: {ids}"

        info = parse_content(mcp_session.call_tool("capsem_info", {"id": new_name}))
        assert info.get("persistent") is True, f"info after persist: {info}"
    finally:
        for candidate in (new_name, vm_id):
            try:
                mcp_session.call_tool("capsem_delete", {"id": candidate})
            except Exception:
                pass


def test_persist_duplicate_name_rejected(fresh_vm, mcp_session):
    """Persisting into an already-used name must fail."""
    taken = fresh_vm()  # already-persistent VM holding the name

    data = parse_content(mcp_session.call_tool("capsem_create", {}))
    ephemeral = data.get("id") or data.get("name")
    try:
        resp = mcp_session.call_tool_raw("capsem_persist", {
            "id": ephemeral,
            "name": taken,
        })
        result = resp.get("result", {})
        assert result.get("isError") is True or "error" in resp, (
            f"expected error on duplicate persist name, got: {resp}"
        )
    finally:
        try:
            mcp_session.call_tool("capsem_delete", {"id": ephemeral})
        except Exception:
            pass


def test_purge_ephemeral_only(fresh_vm, mcp_session):
    """purge with all=false removes ephemerals, preserves persistent."""
    named = fresh_vm()  # persistent

    eph_data = parse_content(mcp_session.call_tool("capsem_create", {}))
    eph_id = eph_data.get("id") or eph_data.get("name")
    assert eph_id

    mcp_session.call_tool("capsem_purge", {"all": False})

    listing = parse_content(mcp_session.call_tool("capsem_list"))
    ids = {s["id"] for s in listing["sandboxes"]}
    assert named in ids, f"persistent VM {named} removed by purge all=false"
    assert eph_id not in ids, f"ephemeral {eph_id} survived purge all=false"


def test_purge_all(isolated_mcp_session):
    """purge with all=true destroys both ephemerals and persistents.

    Runs on its own capsem-service instance: `purge all=true` wipes every
    sandbox on the service, so sharing the fixture would destroy the
    session-scoped shared_vm on the same xdist worker.
    """
    named_a = f"purge-a-{uuid.uuid4().hex[:6]}"
    named_b = f"purge-b-{uuid.uuid4().hex[:6]}"
    isolated_mcp_session.call_tool("capsem_create", {"name": named_a})
    isolated_mcp_session.call_tool("capsem_create", {"name": named_b})
    eph_data = parse_content(isolated_mcp_session.call_tool("capsem_create", {}))
    eph_id = eph_data.get("id") or eph_data.get("name")
    assert eph_id

    isolated_mcp_session.call_tool("capsem_purge", {"all": True})

    listing = parse_content(isolated_mcp_session.call_tool("capsem_list"))
    ids = {s["id"] for s in listing["sandboxes"]}
    for removed in (named_a, named_b, eph_id):
        assert removed not in ids, f"{removed} survived purge all=true: {ids}"


def test_isolated_mcp_session_does_not_affect_shared_service(
    mcp_session, isolated_mcp_session
):
    """isolated_mcp_session must target a different service than mcp_session.

    Pins the invariant behind the fix: destructive tests (test_purge_all)
    run on their own service, so session-scoped fixtures (shared_vm) on
    the same xdist worker survive.
    """
    bystander = f"bystand-{uuid.uuid4().hex[:6]}"
    mcp_session.call_tool("capsem_create", {"name": bystander})
    try:
        isolated_mcp_session.call_tool("capsem_purge", {"all": True})

        listing = parse_content(mcp_session.call_tool("capsem_list"))
        ids = {s["id"] for s in listing["sandboxes"]}
        assert bystander in ids, (
            f"{bystander} destroyed by isolated purge -- services are not isolated"
        )
    finally:
        try:
            mcp_session.call_tool("capsem_delete", {"id": bystander})
        except Exception:
            pass
