"""Saved skill routes enforce scope and exact revisions at the native gateway."""

import secrets
import uuid
from contextlib import closing

import pytest
from helpers.gateway import TcpHttpClient
from helpers.service_gateway import ServiceGateway

from tests.ironbank.kingslanding.test_environment_startup import save
from tests.ironbank.kingslanding.test_run import service

__all__ = ["service"]
pytestmark = pytest.mark.integration


def test_native_skill_routes_enforce_environment_scope_and_exact_revision(service):
    saved = save(service, {"name": "skills-visible"})
    other = save(service, {"name": "skills-outside-scope"})
    root = f"/environments/{saved['environment_id']}/skills"
    gateway = ServiceGateway(service)
    gateway.start()
    try:
        with closing(TcpHttpClient(gateway.base_url, gateway.token)) as operator:
            status, inventory = operator.call_json("GET", f"{root}/list?revision_id={saved['revision_id']}")
            assert status == 200, inventory
            assert inventory == {
                "revision": saved, "skills": [], "discovery_fresh": False, "next_cursor": None,
            }
            assert operator.call_json("GET", f"{root}/list?revision_id={other['revision_id']}")[0] == 409
            assert operator.call_json("GET", f"{root}/list?limit=65")[0] == 400
            assert operator.call_json("GET", f"{root}/unknown/info")[0] == 404

            secret = secrets.token_hex(32)
            status, grant = operator.call_json("POST", "/agent-access", {
                "request_id": str(uuid.uuid4()), "ownership_token": secret, "lease_seconds": 300,
                "scope": {"operations": ["inspect"], "environment_ids": [saved["environment_id"]]},
            })
            assert status == 200, grant
            token = f"{grant['access']['id']}.{secret}"
            with closing(TcpHttpClient(gateway.base_url, token)) as agent:
                assert agent.call_json("GET", f"{root}/list") == (200, inventory)
                assert agent.call_json("GET", f"{root}/unknown/info")[0] == 404
                outside = f"/environments/{other['environment_id']}/skills"
                assert agent.call_json("GET", f"{outside}/list")[0] == 403
                assert agent.call_json("GET", f"{outside}/unknown/info")[0] == 403
    finally:
        gateway.stop()
