"""Native global rule and MCP discovery require Discover delegation."""

import secrets
import uuid
from contextlib import closing

import pytest
from helpers.gateway import TcpHttpClient
from helpers.service_gateway import ServiceGateway

from tests.ironbank.kingslanding.test_run import service

__all__ = ["service"]
pytestmark = pytest.mark.integration


@pytest.mark.parametrize("catalog", ["rules", "mcps"])
def test_native_catalog_routes_require_discover_scope(service, catalog):
    gateway = ServiceGateway(service)
    gateway.start()
    try:
        paths = (f"/{catalog}/list?limit=0", f"/{catalog}/review/info?version=1.0.0&snapshot=invalid")
        with closing(TcpHttpClient(gateway.base_url, gateway.token)) as operator:
            for path in paths:
                assert operator.call_json("GET", path)[0] == 400
            for operation, expected_status in (("inspect", 403), ("discover", 400)):
                secret = secrets.token_hex(32)
                status, response = operator.call_json("POST", "/agent-access", {
                    "request_id": str(uuid.uuid4()), "ownership_token": secret, "lease_seconds": 300,
                    "scope": {"operations": [operation]},
                })
                assert status == 200, response
                token = f"{response['access']['id']}.{secret}"
                with closing(TcpHttpClient(gateway.base_url, token)) as agent:
                    for path in paths:
                        assert agent.call_json("GET", path)[0] == expected_status
    finally:
        gateway.stop()
