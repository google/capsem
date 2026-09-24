"""Gateway route contract for UI/TUI-consumed service endpoints.

The frontend and TUI talk to capsem-service through capsem-gateway. If a
service route is not explicitly forwarded here, the UI sees a gateway 404 even
when the service owns the endpoint.
"""

from __future__ import annotations

import json

from helpers.gateway import TcpHttpClient


def _json_route(client: TcpHttpClient, path: str) -> dict:
    status, body = client.get_status_and_body(path)
    assert status == 200, (path, status, body)
    return json.loads(body)


def test_gateway_does_not_forward_retired_snapshot_routes(gw_client: TcpHttpClient) -> None:
    # The mock service still answers these paths; only the gateway can 404.
    vm = "/vms/11111111-1111-4111-8111-111111111111"
    for path in (f"{vm}/snapshots/status", f"{vm}/changes?checkpoint=cp-0"):
        status, body = gw_client.get_status_and_body(path)
        assert status == 404, (path, status, body)


def test_gateway_forwards_update_status_for_update_surfaces(
    gw_client: TcpHttpClient,
) -> None:
    status = _json_route(gw_client, "/update/status")

    assert status["channel_url"] == "https://release.capsem.org/assets/stable/manifest.json"
    assert status["channel_hash"] == (
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
    )
    assert status["validation_status"] == "valid"
    assert status["binary"]["state"] == "update_available"
    assert status["binary"]["update_available"] is True
    assert status["assets"]["latest"] == "2026.0628.1"
    assert status["profiles"]["latest"] == "profiles-2030.0101.1"
    assert status["images"]["state"] == "not_published"
