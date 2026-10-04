"""Settings-scope UI route contract through the real HTTP gateway.

The asset, plugin and MCP pages talk to capsem-service through
capsem-gateway, not directly over the service UDS. These tests keep that
boundary honest: a service route that is not explicitly forwarded by the
gateway is a user-visible 404.
"""

from __future__ import annotations

import json

import pytest
from helpers.gateway import GatewayInstance, TcpHttpClient
from helpers.service import ServiceInstance

pytestmark = [pytest.mark.gateway, pytest.mark.integration]


def _json_status(client: TcpHttpClient, path: str) -> tuple[int, object]:
    status, body = client.get_status_and_body(path)
    payload = json.loads(body) if body else {}
    return status, payload


def test_settings_routes_are_forwarded_through_gateway() -> None:
    svc = ServiceInstance()
    gw: GatewayInstance | None = None
    try:
        svc.start()
        gw = GatewayInstance(uds_path=svc.uds_path)
        gw.start()
        client = TcpHttpClient(gw.base_url, gw.token)

        route_expectations = {
            "/assets/status": {"ready", "downloading", "current_arch", "assets", "errors", "manifest"},
            "/plugins/list": {"plugins"},
            "/plugins/credential_broker/credentials/info": {
                "plugin_id",
                "store",
                "inventory",
                "grants",
                "corp_constraints",
            },
            "/mcp/info": {"server_count", "manual_server_count", "builtin_local_enabled"},
            "/mcp/default/info": {"action", "source", "rule_id"},
        }
        for path, required_keys in route_expectations.items():
            status, payload = _json_status(client, path)
            assert status == 200, f"{path} returned {status}: {payload}"
            assert isinstance(payload, dict), (path, payload)
            assert required_keys <= payload.keys(), (path, payload.keys())

        status, servers = _json_status(client, "/mcp/servers/list")
        assert status == 200, servers
        assert isinstance(servers, list)

        status, payload = client.call_json(
            "POST", "/plugins/credential_broker/credentials/reload", {}, timeout=30
        )
        assert status == 200, payload
        assert "scope" not in payload
        assert payload["plugin_id"] == "credential_broker"
        assert {
            "backend",
            "ready",
            "status",
            "cached_count",
            "last_hydrated_count",
            "last_hydrated_unix_ms",
            "last_error",
        } <= payload["store"].keys()
    finally:
        if gw is not None:
            gw.stop()
        svc.stop()
