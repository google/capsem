"""Ironbank profile UI route contract through service and gateway.

This is the black-box guard for the "API error 404" class of UI bugs: every
profile-facing surface the UI uses must exist for every shipped profile through
both the service UDS route and the authenticated gateway route.
"""

from __future__ import annotations

import json
from typing import Any

from helpers.gateway import GatewayInstance, TcpHttpClient
from helpers.route_matrix import RouteSpec, assert_profile_route_matrix
from helpers.service import ServiceInstance


PROFILES = ("code", "co-work")


def _service_request(client: Any, spec: RouteSpec) -> Any:
    if spec.method == "GET":
        return client.get(spec.path, timeout=30)
    if spec.method == "POST":
        return client.post(spec.path, spec.body, timeout=30)
    raise AssertionError(f"unsupported service route method: {spec.method}")


def _gateway_request(client: TcpHttpClient, spec: RouteSpec) -> Any:
    status, body = client.get_status_and_body(
        spec.path,
        timeout=30,
        extra_headers={"Content-Type": "application/json"},
    ) if spec.method == "GET" else _gateway_post_status_and_body(client, spec)
    assert status == 200, (spec.path, status, body)
    payload = json.loads(body)
    assert not (isinstance(payload, dict) and payload.get("error")), (spec.path, payload)
    return payload


def _gateway_post_status_and_body(client: TcpHttpClient, spec: RouteSpec) -> tuple[int, str]:
    import subprocess

    cmd = [
        "curl",
        "-s",
        "-S",
        "-X",
        "POST",
        "-H",
        f"Authorization: Bearer {client.token}",
        "-H",
        "Content-Type: application/json",
        "-d",
        json.dumps(spec.body or {}),
        "-w",
        "\n%{http_code}",
        "--max-time",
        "30",
        f"{client.base_url}{spec.path}",
    ]
    result = subprocess.run(cmd, capture_output=True, text=True, timeout=35)
    assert result.returncode == 0, (spec.path, result.stderr)
    body, _, status_text = result.stdout.rpartition("\n")
    return int(status_text), body


def test_profile_ui_routes_exist_through_service_and_gateway() -> None:
    service = ServiceInstance()
    gateway: GatewayInstance | None = None
    try:
        service.start()
        gateway = GatewayInstance(uds_path=service.uds_path)
        gateway.start()
        service_client = service.client()
        gateway_client = TcpHttpClient(gateway.base_url, gateway.token)

        for client_name, request in (
            ("service", lambda spec: _service_request(service_client, spec)),
            ("gateway", lambda spec: _gateway_request(gateway_client, spec)),
        ):
            profiles = service_client.get("/profiles/list", timeout=30)
            listed_ids = {profile["id"] for profile in profiles["profiles"]}
            assert set(PROFILES) <= listed_ids, (client_name, listed_ids)
            assert_profile_route_matrix(profiles=PROFILES, request=request)
    finally:
        if gateway is not None:
            gateway.stop()
        service.stop()
