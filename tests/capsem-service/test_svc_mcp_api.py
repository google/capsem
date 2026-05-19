"""Profile V2 MCP connector service API."""

import uuid

import pytest

pytestmark = pytest.mark.integration


class TestMcpConnectors:
    def test_connectors_returns_profile_v2_shape(self, client):
        resp = client.get("/mcp/connectors")
        assert resp["mode"] == "settings_profiles_v2"
        assert isinstance(resp["profile_id"], str)
        assert isinstance(resp["connectors"], list)
        for connector in resp["connectors"]:
            for key in ("id", "source_profile", "source", "direct", "editable", "connector"):
                assert key in connector, f"connector missing '{key}': {connector}"
            assert isinstance(connector["direct"], bool)
            assert isinstance(connector["editable"], bool)
            assert isinstance(connector["connector"], dict)

    def test_connector_create_list_delete_roundtrip(self, client):
        connector_id = f"pytest-{uuid.uuid4().hex[:8]}"
        created = client.post(
            "/mcp/connectors",
            {
                "id": connector_id,
                "enabled": True,
                "connector_type": "mcp",
                "credential_refs": ["pytest-token"],
                "allowed_tools": ["repo.read"],
            },
        )
        assert created["id"] == connector_id
        assert created["connector"]["enabled"] is True
        assert created["connector"]["credential_refs"] == ["pytest-token"]
        assert created["connector"]["allowed_tools"] == ["repo.read"]

        listed = client.get("/mcp/connectors")
        by_id = {item["id"]: item for item in listed["connectors"]}
        assert connector_id in by_id
        assert by_id[connector_id]["editable"] is True

        deleted = client.delete(f"/mcp/connectors/{connector_id}")
        assert deleted == {
            "mode": "settings_profiles_v2",
            "profile_id": listed["profile_id"],
            "connector_id": connector_id,
            "removed": True,
        }

        listed_after = client.get("/mcp/connectors")
        assert connector_id not in {item["id"] for item in listed_after["connectors"]}
