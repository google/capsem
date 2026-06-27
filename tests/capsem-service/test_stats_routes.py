"""Service stats route contract.

These are the lightweight service-level stats gates. Deep session.db projection
coverage lives in tests/ironbank/test_stats_detail_contract.py.
"""

from __future__ import annotations

import pytest

from helpers.service import ServiceInstance


pytestmark = pytest.mark.integration


def test_global_stats_route_exposes_route_owned_shape(client) -> None:
    payload = client.get("/stats")
    assert set(payload) >= {
        "global",
        "sessions",
        "top_providers",
        "top_tools",
        "top_mcp_tools",
    }
    assert isinstance(payload["global"], dict)
    assert isinstance(payload["sessions"], list)
    assert isinstance(payload["top_providers"], list)
    assert isinstance(payload["top_tools"], list)
    assert isinstance(payload["top_mcp_tools"], list)


def test_security_detection_enforcement_service_routes_are_db_backed_empty_lists() -> None:
    service = ServiceInstance()
    service.start()
    client = service.client()
    try:
        for path in ("/security/latest", "/detection/latest", "/enforcement/latest"):
            payload = client.get(path)
            assert payload == [], path

        for path in ("/security/status", "/detection/status", "/enforcement/status"):
            payload = client.get(path)
            assert payload["total"] == 0, path
            assert payload["sessions"] == [], path
    finally:
        service.stop()
