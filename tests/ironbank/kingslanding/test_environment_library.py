"""Saved environment discovery through the native authenticated gateway."""

import secrets
import uuid
from contextlib import closing

import pytest
from helpers.gateway import TcpHttpClient
from helpers.service import vm_name
from helpers.service_gateway import ServiceGateway

from tests.ironbank.kingslanding.test_environment_startup import create, request, save
from tests.ironbank.kingslanding.test_run import service

__all__ = ["service"]
pytestmark = pytest.mark.integration


def test_environment_library_pages_verified_heads_and_keeps_delegation_exact(service):
    first = save(
        service,
        {
            "name": "research",
            "description": "Métier data",
            "variables": {"LIBRARY_PRIVATE_MARKER": "one"},
        },
    )
    second = save(service, {"name": "reporting", "description": "Report generation"})
    vm = create(service, first, vm_name("library-count"))
    gateway = ServiceGateway(service)
    gateway.start()
    route = "/environments/list"
    try:
        with closing(TcpHttpClient(gateway.base_url, gateway.token)) as operator:
            status, page = operator.call_json("GET", f"{route}?limit=1")
            assert status == 200, page
            assert len(page["environments"]) == 1 and page["next_cursor"] is not None
            page2 = operator.get(f"{route}?limit=1&after={page['next_cursor']}")
            assert len(page2["environments"]) == 1 and page2["next_cursor"] is None
            items = page["environments"] + page2["environments"]
            assert [item["environment"] for item in items] == sorted(
                [first, second],
                key=lambda reference: reference["environment_id"],
            )
            assert "LIBRARY_PRIVATE_MARKER" not in str(items)
            search = operator.get(f"{route}?search=M%C3%89TIER")
            found = search["environments"]
            assert len(found) == 1 and found[0]["environment"] == first
            assert found[0]["summary"]["name"] == "research"
            assert found[0]["instance_count"] == 1
            request(service, "POST", f"/vms/{vm}/stop", {})
            assert (
                operator.get(f"{route}?search=research")["environments"][0]["instance_count"] == 1
            )
            for query in (
                "limit=0",
                "limit=65",
                "after=unknown",
                "search=%00",
                "unexpected=parameter",
            ):
                assert operator.call_json("GET", f"{route}?{query}")[0] in (400, 422)

            secret = secrets.token_hex(32)
            status, issued = operator.call_json(
                "POST",
                "/agent-access",
                {
                    "request_id": str(uuid.uuid4()),
                    "ownership_token": secret,
                    "lease_seconds": 300,
                    "scope": {
                        "operations": ["inspect"],
                        "environment_ids": [first["environment_id"]],
                    },
                },
            )
            assert status == 200, issued
            grant_id = issued["access"]["id"]
            with closing(TcpHttpClient(gateway.base_url, f"{grant_id}.{secret}")) as agent:
                assert agent.get(route) == {
                    "environments": [{**found[0], "instance_count": None}],
                    "next_cursor": None,
                }
                assert agent.get(f"{route}?search=reporting")["environments"] == []
                assert operator.call_json("DELETE", f"/agent-access/{grant_id}")[0] == 200
                assert agent.call_json("GET", route)[0] in (401, 403)
    finally:
        gateway.stop()
