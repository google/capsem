"""Profile UI route contract through the real HTTP gateway.

The profile page talks to capsem-service through capsem-gateway, not directly
over the service UDS. These tests keep that boundary honest: a service route
that is not explicitly forwarded by the gateway is a user-visible 404.
"""

from __future__ import annotations

import json
import subprocess

import pytest

from helpers.constants import CODE_PROFILE_ID
from helpers.gateway import GatewayInstance, TcpHttpClient
from helpers.service import ServiceInstance

pytestmark = [pytest.mark.gateway, pytest.mark.integration]


def _json_status(client: TcpHttpClient, path: str) -> tuple[int, dict]:
    status, body = client.get_status_and_body(path)
    payload = json.loads(body) if body else {}
    return status, payload


def _post_json_status(client: TcpHttpClient, path: str) -> tuple[int, dict]:
    cmd = [
        "curl",
        "-s",
        "-S",
        "-X",
        "POST",
        "-H",
        "Content-Type: application/json",
        "-H",
        f"Authorization: Bearer {client.token}",
        "-d",
        "{}",
        "-w",
        "\n%{http_code}",
        "--max-time",
        "30",
        f"{client.base_url}{path}",
    ]
    result = subprocess.run(cmd, capture_output=True, text=True, timeout=35)
    assert result.returncode == 0, result.stderr
    body, status_text = result.stdout.rsplit("\n", 1)
    return int(status_text), json.loads(body) if body else {}


def test_profile_overview_routes_are_forwarded_through_gateway() -> None:
    svc = ServiceInstance()
    gw: GatewayInstance | None = None
    try:
        svc.start()
        gw = GatewayInstance(uds_path=svc.uds_path)
        gw.start()
        client = TcpHttpClient(gw.base_url, gw.token)

        profile_id = CODE_PROFILE_ID
        route_expectations = {
            f"/profiles/{profile_id}/info": {"profile", "obom"},
            f"/profiles/{profile_id}/plugins/credential_broker/credentials/info": {
                "scope",
                "plugin_id",
                "store",
                "inventory",
                "grants",
                "corp_constraints",
            },
            f"/profiles/{profile_id}/assets/status": {
                "profile_id",
                "ready",
                "assets",
                "missing_assets",
                "invalid_assets",
                "manifest",
            },
            f"/profiles/{profile_id}/enforcement/rules/list": {
                "profile_id",
                "rules",
            },
            f"/profiles/{profile_id}/detection/rules/list": {
                "profile_id",
                "rules",
            },
        }

        for path, required_keys in route_expectations.items():
            status, payload = _json_status(client, path)
            assert status == 200, f"{path} returned {status}: {payload}"
            assert required_keys <= payload.keys(), (path, payload.keys())

        status, payload = _post_json_status(
            client,
            f"/profiles/{profile_id}/plugins/credential_broker/credentials/reload",
        )
        assert status == 200, payload
        assert payload["scope"]["profile_id"] == profile_id
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
