"""Saved environment discovery through the native authenticated gateway."""

import secrets
import tomllib
import uuid
from contextlib import closing

import pytest
from helpers.gateway import TcpHttpClient
from helpers.service import vm_name
from helpers.service_gateway import ServiceGateway

from tests.fixtures.oci.registry import publish_catalog, registry
from tests.ironbank.kingslanding.test_environment_startup import create, request, save
from tests.ironbank.kingslanding.test_image_catalog import entry, use_catalog
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


def test_environment_create_replays_the_committed_pin_without_catalog_after_restart(service, tmp_path):
    artifacts = {}
    with registry(tmp_path, artifacts=artifacts) as (image, certificate, requests):
        authority = image.split("/", 1)[0]
        publish_catalog(artifacts, "library/catalog", "stable", entry(image))
        use_catalog(service, f"{authority}/library/catalog:stable", certificate)
        gateway = ServiceGateway(service)
        route = "/environments/create"
        payload = {
            "request_id": str(uuid.uuid4()),
            "reason": "Save the admitted Redis image",
            "specification": {"name": "restart-create-replay", "image": {"reference": "redis"}},
        }
        gateway.start()
        try:
            with closing(TcpHttpClient(gateway.base_url, gateway.token)) as operator:
                supplied_pin = {
                    **payload["specification"],
                    "image": {"reference": "redis", "resolved_digest": image.rsplit("@", 1)[1]},
                }
                assert operator.call_json("POST", route, {**payload, "specification": supplied_pin})[0] == 422
                status, receipt = operator.call_json("POST", route, payload)
                assert status == 200 and receipt["decision"] == "approved" and receipt["application"] == "applied"
                selected = receipt["result"]
                root = f"/environments/{selected['environment_id']}"
                revision = f"{root}/revisions/{selected['revision_id']}/info"
                before = operator.get(revision)
                assert tomllib.loads(before["canonical_toml"])["image"]["resolved_digest"] == image.rsplit("@", 1)[1]
        finally:
            gateway.stop()

        service.stop(cleanup=False)
        use_catalog(service, f"{authority}/library/withdrawn-catalog:stable", certificate)
        service.start()
        gateway.start()
        try:
            with closing(TcpHttpClient(gateway.base_url, gateway.token)) as operator:
                before_replay = len(requests)
                assert operator.call_json("POST", route, payload) == (200, receipt)
                assert operator.call_json("POST", route, {**payload, "reason": "Different intent"})[0] == 409
                assert operator.get(revision) == before
                assert len(requests) == before_replay, requests[before_replay:]

                fresh = {**payload, "request_id": str(uuid.uuid4())}
                status, refused = operator.call_json("POST", route, fresh)
                assert (status, refused) == (400, {"error": "environment source resolution failed"})
                assert any(path.startswith("/v2/library/withdrawn-catalog/")
                           for path in requests[before_replay:])
                rows = operator.get("/environments/list?search=restart-create-replay")["environments"]
                assert [row["environment"] for row in rows] == [selected]
                assert operator.call_json("GET", f"/requests/{fresh['request_id']}/info")[0] == 404
        finally:
            gateway.stop()
