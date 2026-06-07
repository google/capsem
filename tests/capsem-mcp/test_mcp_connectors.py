"""Profile V2 MCP server tools exposed by capsem-mcp."""

import uuid

import pytest

pytestmark = pytest.mark.mcp


def _content_text(resp):
    content = resp.get("content", [])
    assert content, f"missing MCP content: {resp}"
    return content[0].get("text", "")


def test_mcp_connectors_add_list_delete_roundtrip(editable_isolated_mcp_session):
    connector_id = f"pytest-{uuid.uuid4().hex[:8]}"

    created = editable_isolated_mcp_session.call_tool(
        "capsem_mcp_add",
        {
            "id": connector_id,
            "type": "stdio",
            "command": "npx",
            "args": ["-y", "@modelcontextprotocol/server-github"],
            "env": {"GITHUB_TOKEN": "env:CAPSEM_GITHUB_TOKEN"},
            "credential_refs": ["pytest-token"],
            "allowed_tools": ["repo.read"],
        },
    )
    assert connector_id in _content_text(created)

    listed = editable_isolated_mcp_session.call_tool("capsem_mcp_connectors")
    listed_text = _content_text(listed)
    assert connector_id in listed_text
    assert "npx" in listed_text
    assert "@modelcontextprotocol/server-github" in listed_text
    assert "pytest-token" in listed_text
    assert "repo.read" in listed_text

    deleted = editable_isolated_mcp_session.call_tool(
        "capsem_mcp_delete",
        {"id": connector_id},
    )
    assert connector_id in _content_text(deleted)

    listed_after = editable_isolated_mcp_session.call_tool("capsem_mcp_connectors")
    assert connector_id not in _content_text(listed_after)


def test_mcp_connector_duplicate_surfaces_service_error(editable_isolated_mcp_session):
    connector_id = f"pytest-{uuid.uuid4().hex[:8]}"
    editable_isolated_mcp_session.call_tool(
        "capsem_mcp_add",
        {"id": connector_id, "type": "stdio", "command": "npx"},
    )

    resp = editable_isolated_mcp_session.call_tool_raw(
        "capsem_mcp_add",
        {"id": connector_id, "type": "stdio", "command": "npx"},
    )
    result = resp.get("result", {})
    assert result.get("isError") is True or "error" in resp, (
        f"expected duplicate connector error, got: {resp}"
    )
