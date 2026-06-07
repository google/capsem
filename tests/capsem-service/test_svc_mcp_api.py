"""Profile V2 MCP server service API."""

import uuid

import pytest

pytestmark = pytest.mark.integration


class TestMcpConnectors:
    def test_servers_returns_profile_v2_shape(self, client):
        resp = client.get("/mcp/connectors")
        assert resp["mode"] == "settings_profiles_v2"
        assert isinstance(resp["profile_id"], str)
        assert isinstance(resp["servers"], list)
        for server in resp["servers"]:
            for key in ("id", "source_profile", "source", "direct", "editable", "server"):
                assert key in server, f"server missing '{key}': {server}"
            assert isinstance(server["direct"], bool)
            assert isinstance(server["editable"], bool)
            assert isinstance(server["server"], dict)

    def test_connector_create_list_delete_roundtrip(self, editable_client):
        client = editable_client
        connector_id = f"pytest-{uuid.uuid4().hex[:8]}"
        created = client.post(
            "/mcp/connectors",
            {
                "id": connector_id,
                "enabled": True,
                "type": "stdio",
                "command": "npx",
                "args": ["-y", "@modelcontextprotocol/server-github"],
                "env": {"GITHUB_TOKEN": "env:CAPSEM_GITHUB_TOKEN"},
                "capsem": {
                    "credential_refs": ["pytest-token"],
                    "allowed_tools": ["repo.read"],
                },
            },
        )
        assert created["id"] == connector_id
        assert created["server"]["enabled"] is True
        assert created["server"]["command"] == "npx"
        assert created["server"]["args"] == ["-y", "@modelcontextprotocol/server-github"]
        assert created["server"]["env"] == {"GITHUB_TOKEN": "env:CAPSEM_GITHUB_TOKEN"}
        assert created["server"]["capsem"]["credential_refs"] == ["pytest-token"]
        assert created["server"]["capsem"]["allowed_tools"] == ["repo.read"]

        listed = client.get("/mcp/connectors")
        by_id = {item["id"]: item for item in listed["servers"]}
        assert connector_id in by_id
        assert by_id[connector_id]["editable"] is True

        deleted = client.delete(f"/mcp/connectors/{connector_id}")
        assert deleted == {
            "mode": "settings_profiles_v2",
            "profile_id": listed["profile_id"],
            "server_id": connector_id,
            "removed": True,
        }

        listed_after = client.get("/mcp/connectors")
        assert connector_id not in {item["id"] for item in listed_after["servers"]}
