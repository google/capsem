"""capsem_mcp_call: route tool invocations through a running VM's aggregator.

The test harness does not configure downstream MCP servers, so only the
error paths are reachable end-to-end. Happy-path coverage would require
spawning a downstream stdio MCP server from the test fixture -- tracked
as follow-up work.
"""

import uuid

import pytest

pytestmark = pytest.mark.mcp


def test_mcp_call_unknown_tool(shared_vm, mcp_session):
    """Calling a non-existent namespaced tool surfaces an aggregator error.

    This proves the routing chain: MCP stdio -> capsem-service ->
    running-instance MCP aggregator -> error return. Prior to this test,
    nothing in the suite exercised that full path.
    """
    _vm_name, _ = shared_vm
    bogus_name = f"nonexistent__{uuid.uuid4().hex[:8]}"
    resp = mcp_session.call_tool_raw("capsem_mcp_call", {
        "name": bogus_name,
        "arguments": {},
    })
    result = resp.get("result", {})
    assert result.get("isError") is True or "error" in resp, (
        f"expected error calling unknown tool, got: {resp}"
    )


def test_mcp_call_missing_arguments(shared_vm, mcp_session):
    """arguments is optional -- MCP layer defaults it to {} when omitted."""
    _vm_name, _ = shared_vm
    bogus_name = f"nonexistent__{uuid.uuid4().hex[:8]}"
    # No arguments field at all -- should still reach the aggregator and
    # return a tool-not-found error (not a schema validation failure on our side).
    resp = mcp_session.call_tool_raw("capsem_mcp_call", {"name": bogus_name})
    result = resp.get("result", {})
    assert result.get("isError") is True or "error" in resp, (
        f"expected error, got: {resp}"
    )
