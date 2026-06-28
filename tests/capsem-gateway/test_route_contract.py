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


def test_gateway_forwards_snapshot_routes_used_by_stats_ui(gw_client: TcpHttpClient) -> None:
    status = _json_route(gw_client, "/vms/11111111-1111-4111-8111-111111111111/snapshots/status")
    assert status["total"] == 1
    assert status["auto_count"] == 1
    assert status["manual_count"] == 0
    assert status["snapshots"][0]["checkpoint"] == "checkpoint-0"
    assert status["snapshots"][0]["origin"] == "auto"

    listing = _json_route(gw_client, "/vms/11111111-1111-4111-8111-111111111111/snapshots/list")
    assert listing["total"] == 1
    assert listing["snapshots"] == status["snapshots"]


def test_gateway_forwards_update_status_for_update_surfaces(
    gw_client: TcpHttpClient,
) -> None:
    status = _json_route(gw_client, "/update/status")

    assert status["channel_url"] == "https://release.capsem.org/health.json"
    assert status["channel_hash"] == (
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
    )
    assert status["validation_status"] == "valid"
    assert status["binary"]["state"] == "update_available"
    assert status["binary"]["update_available"] is True
    assert status["assets"]["latest"] == "2026.0628.1"
    assert status["profiles"]["latest"] == "profiles-2030.0101.1"
    assert status["images"]["state"] == "not_published"
