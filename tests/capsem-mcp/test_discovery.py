"""MCP tool discovery and schema validation."""

import pytest

pytestmark = pytest.mark.mcp

EXPECTED_TOOLS = {
    "capsem_create", "capsem_list", "capsem_info",
    "capsem_exec", "capsem_read_file", "capsem_write_file",
    "capsem_inspect_schema", "capsem_inspect", "capsem_delete",
}


def test_all_tools_discovered(mcp_session):
    """All 9 capsem tools must appear in tools/list."""
    resp = mcp_session.request("tools/list")
    tools = {t["name"] for t in resp["result"]["tools"]}
    missing = EXPECTED_TOOLS - tools
    assert not missing, f"Missing tools: {missing}"


def test_tool_schemas_have_type(mcp_session):
    """Every tool must have an inputSchema with a type field."""
    resp = mcp_session.request("tools/list")
    for tool in resp["result"]["tools"]:
        schema = tool.get("inputSchema", {})
        assert "type" in schema, f"{tool['name']} missing inputSchema.type"


def test_tool_descriptions_nonempty(mcp_session):
    """Every tool must have a non-empty description."""
    resp = mcp_session.request("tools/list")
    for tool in resp["result"]["tools"]:
        assert tool.get("description"), f"{tool['name']} has no description"


def test_create_schema_fields(mcp_session):
    """capsem_create schema must declare name, ramMb, cpuCount."""
    resp = mcp_session.request("tools/list")
    create = next(t for t in resp["result"]["tools"] if t["name"] == "capsem_create")
    props = create["inputSchema"].get("properties", {})
    assert "name" in props, "Missing 'name' in create schema"
    assert "ramMb" in props, "Missing 'ramMb' in create schema"
    assert "cpuCount" in props, "Missing 'cpuCount' in create schema"


def test_exec_schema_fields(mcp_session):
    """capsem_exec schema must require id and command."""
    resp = mcp_session.request("tools/list")
    exec_tool = next(t for t in resp["result"]["tools"] if t["name"] == "capsem_exec")
    props = exec_tool["inputSchema"].get("properties", {})
    assert "id" in props
    assert "command" in props
