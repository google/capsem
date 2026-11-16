"""Native transport provenance: a bearer/header cannot mint a gateway seat."""

from __future__ import annotations

import json
import time

import pytest
from helpers.gateway import TcpHttpClient, _runtime_connection
from helpers.service import ServiceInstance
from helpers.uds_client import UdsHttpClient

pytestmark = pytest.mark.integration


def test_service_private_gateway_status_requires_the_brokered_channel():
    service = ServiceInstance(isolated_corp_config=True)
    try:
        service.start()
        deadline = time.monotonic() + 15
        connection = None
        while time.monotonic() < deadline:
            connection = _runtime_connection(service.tmp_dir)
            if connection is not None:
                break
            time.sleep(0.05)
        assert connection is not None, "service-owned gateway did not publish readiness"
        token, port = connection
        gateway = TcpHttpClient(f"http://127.0.0.1:{port}", token)
        # This cached public route fetches the real private service snapshot.
        # Both a fresh connection and the following pooled request must work.
        for _ in range(2):
            status, _, data = gateway.call("GET", "/vms/list")
            assert status == 200, data.decode(errors="replace")
            assert json.loads(data)["sandboxes"] == []
        direct = UdsHttpClient(service.uds_path)
        for headers in (
            {},
            {
                "Authorization": f"Bearer {token}",
                "x-capsem-actor": "operator",
                "x-capsem-gateway": "true",
            },
        ):
            status, _, _ = direct.call("GET", "/internal/gateway/status", headers=headers)
            assert status == 403, "direct peer forged gateway transport provenance"
        claimed_operator = {
            "Authorization": f"Bearer {token}",
            "x-capsem-actor": "operator",
            "Content-Type": "application/json",
        }
        for method, route in [
            ("GET", "/vms/list"),
            ("POST", "/credentials/inject"),
            ("PATCH", "/settings/edit"),
            ("PATCH", "/plugins/absent/edit"),
            ("PUT", "/corp/edit"),
            ("PATCH", "/mcp/default/edit"),
            ("POST", "/vms/absent/security/asks/absent/resolve"),
        ]:
            status, _, _ = direct.call(
                method,
                route,
                body=b'{"actor":{"kind":"operator"}}',
                headers=claimed_operator,
            )
            assert status == 403, f"raw service route accepted ambient authority: {method} {route}"
        status, _, _ = gateway.call("GET", "/vms/list", use_auth=False)
        assert status == 401
    finally:
        service.stop()
