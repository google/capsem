"""Canonical revisions and TOML admission use the operator's real gateway."""

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


def test_exact_revision_history_and_toml_edit_share_the_committed_path(service):
    old = save(service, {"name": "tracked-work", "variables": {"REVISION_VALUE": "original"}})
    environment = f"/environments/{old['environment_id']}"
    old_path = f"{environment}/revisions/{old['revision_id']}"
    canonical_path = service.home_dir / "environments" / old["environment_id"] / "environment.toml"
    canonical = canonical_path.read_text()

    gateway = ServiceGateway(service)
    gateway.start()
    try:
        with closing(TcpHttpClient(gateway.base_url, gateway.token)) as operator:
            status, historical = operator.call_json("GET", old_path + "/info")
            assert status == 200, historical
            assert historical["canonical_toml"] == canonical
            assert historical["availability"] == "complete"
            assert historical["summary"]["environment"] == old
            assert historical["partial_specification"] is None

            text = canonical.replace('REVISION_VALUE = "original"', 'REVISION_VALUE = "edited"')
            assert text != canonical
            status, draft = operator.call_json("POST", old_path + "/validate", {"toml": text})
            assert status == 200, draft
            assert draft["base"] == old and draft["specification"]["variables"] == {
                "REVISION_VALUE": "edited"
            }
            assert canonical_path.read_text() == canonical
            moved = canonical.replace(old["revision_id"], str(uuid.uuid4()))
            assert operator.call_json("POST", old_path + "/validate", {"toml": moved})[0] == 400
            assert (
                operator.call_json("POST", old_path + "/validate", {"toml": "name = [invalid"})[0]
                == 400
            )

            status, result = operator.call_json(
                "POST",
                environment + "/changes",
                {
                    "request_id": str(uuid.uuid4()),
                    "base_revision_id": old["revision_id"],
                    "specification": draft["specification"],
                    "reason": "Approve the validated TOML change",
                },
            )
            assert status == 200 and result["application"] == "applied", result
            new = result["result"]
            assert new != old and 'REVISION_VALUE = "edited"' in canonical_path.read_text()
            first = operator.get(environment + "/revisions/list?limit=1")
            assert [row["environment"] for row in first["revisions"]] == [new]
            assert first["revisions"][0]["previous_revision_id"] == old["revision_id"]
            older = operator.get(
                f"{environment}/revisions/list?limit=1&before={first['next_cursor']}"
            )
            assert [row["environment"] for row in older["revisions"]] == [old]
            assert older["next_cursor"] is None
            assert operator.get(old_path + "/info")["canonical_toml"] == canonical
            assert operator.call_json("POST", old_path + "/validate", {"toml": text})[0] == 409

            secret = secrets.token_hex(32)
            status, grant = operator.call_json(
                "POST",
                "/agent-access",
                {
                    "request_id": str(uuid.uuid4()),
                    "ownership_token": secret,
                    "lease_seconds": 300,
                    "scope": {
                        "operations": ["inspect"],
                        "environment_ids": [old["environment_id"]],
                    },
                },
            )
            assert status == 200, grant
            with closing(
                TcpHttpClient(gateway.base_url, f"{grant['access']['id']}.{secret}")
            ) as agent:
                assert agent.call_json("GET", environment + "/info")[0] == 200
                assert agent.call_json("GET", environment + "/revisions/list")[0] == 403
                assert agent.call_json("GET", old_path + "/info")[0] == 403
                assert agent.call_json("POST", old_path + "/validate", {"toml": text})[0] == 403
    finally:
        gateway.stop()
