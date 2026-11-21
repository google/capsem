"""An approved change adopts a fork in exactly one persistent VM."""

import secrets
import uuid

import pytest
from helpers.gateway import TcpHttpClient
from helpers.service import exec_output_text, vm_name
from helpers.service_gateway import ServiceGateway

from tests.ironbank.kingslanding.test_environment_startup import create, request, save
from tests.ironbank.kingslanding.test_run import service

__all__ = ["service"]
pytestmark = pytest.mark.integration


@pytest.mark.parametrize("deferred", [False, True], ids=["live", "live-and-next-start"])
def test_approval_applies_live_policy_and_saves_deferred_configuration(service, deferred):
    specification = {"name": "reviewed", "variables": {"CAPSEM_REVIEW_MARKER": "original"}}
    base = save(service, specification)
    owner = create(service, base, vm_name("review-owner"))
    sibling = create(service, base, vm_name("review-sibling"))
    owner_path = f"/vms/{owner}/info"
    sibling_path = f"/vms/{sibling}/info"
    original = request(service, "GET", owner_path)["environment"]
    sibling_binding = request(service, "GET", sibling_path)["environment"]
    gateway = ServiceGateway(service)
    gateway.start()
    operator = TcpHttpClient(gateway.base_url, gateway.token)
    agent = None
    try:
        secret = secrets.token_hex(32)
        grant_status, grant = operator.call_json("POST", "/agent-access", {
            "request_id": str(uuid.uuid4()),
            "ownership_token": secret,
            "lease_seconds": 300,
            "scope": {
                "operations": ["propose", "cancel_own"],
                "sessions": [{"session_id": owner, "owner_generation": original["owner_generation"]}],
            },
        })
        assert grant_status == 200, grant
        agent = TcpHttpClient(gateway.base_url, f"{grant['access']['id']}.{secret}")
        proposal_path = f"/environments/{base['environment_id']}/changes"
        proposed = {
            **specification,
            "variables": {"CAPSEM_REVIEW_MARKER": "approved" if deferred else "original"},
            "access": {"custom": {"scope": {
                "name": "Scoped endpoint", "action": "block",
                "match": 'http.host == "scoped.example"',
            }}},
        }

        def submit():
            request_id = str(uuid.uuid4())
            status, result = agent.call_json("POST", proposal_path, {
                "request_id": request_id,
                "base_revision_id": base["revision_id"],
                "specification": proposed,
                "reason": "update workspace configuration",
            })
            assert status == 200 and result["decision"] == "pending"
            info_path = f"/requests/{request_id}/info"
            detail = operator.get(info_path)
            assert result["observed_revision"] == detail["observed_revision"]
            return f"/requests/{request_id}/resolve", info_path, result["observed_revision"]

        for decision, expected in (("deny", "denied"), ("cancel", "cancelled")):
            path, info_path, observed = submit()
            actor = operator if decision == "deny" else agent
            body = {"observed_revision": observed, "decision": decision, "reason": "keep current settings"}
            status, receipt = actor.call_json("POST", path, body)
            assert status == 200, receipt
            assert receipt["decision"] == expected and receipt["result"] is None
            assert actor.call_json("POST", path, body) == (200, receipt)
            assert operator.get(info_path)["summary"]["decision"] == expected
            assert operator.get(owner_path)["environment"] == original

        path, info_path, observed = submit()
        approved = {"observed_revision": observed, "decision": "approve", "reason": "apply to the reviewed VM"}
        assert agent.call_json("POST", path, approved)[0] == 403
        wrong = "0" * 12 if observed != "0" * 12 else "1" * 12
        assert operator.call_json("POST", path, {**approved, "observed_revision": wrong})[0] == 409
        status, receipt = operator.call_json("POST", path, approved)
        assert status == 200, receipt
        expected_application = "pending" if deferred else "applied"
        assert receipt["decision"] == "approved"
        assert receipt["application"] == expected_application
        fork = receipt["result"]
        assert fork["environment_id"] != base["environment_id"]
        assert operator.call_json("POST", path, approved) == (200, receipt)
        assert operator.get(info_path)["summary"]["application"] == expected_application
        applied = operator.get(owner_path)["environment"]
        assert applied["owner_generation"] == original["owner_generation"]
        assert applied["boot"] == base and applied["desired"] == applied["live_policy"] == fork
        assert applied["pending_restart"] is deferred
        assert applied["policy_digest"] != original["policy_digest"]
        assert operator.get(sibling_path)["environment"] == sibling_binding

        document = operator.get(f"/environments/{fork['environment_id']}/info")["document"]
        assert document["identity"]["parent_environment_id"] == base["environment_id"]
        assert document["identity"]["parent_revision_id"] == base["revision_id"]
        shared = operator.get(f"/environments/{base['environment_id']}/info")["document"]
        assert shared["identity"]["revision_id"] == base["revision_id"]

        def marker(vm_id):
            result = request(service, "POST", f"/vms/{vm_id}/exec", {
                "command": 'printf "%s" "$CAPSEM_REVIEW_MARKER"',
            })
            assert result["exit_code"] == 0, result
            return exec_output_text(result)

        assert marker(owner) == marker(sibling) == "original"
        request(service, "POST", f"/vms/{owner}/stop", {})
        assert operator.get(owner_path)["environment"]["desired"] == fork
        request(service, "POST", f"/vms/{owner}/start", {})
        restarted = operator.get(owner_path)["environment"]
        assert restarted["owner_generation"] != original["owner_generation"]
        assert restarted["boot"] == restarted["live_policy"] == restarted["desired"] == fork
        assert restarted["pending_restart"] is False
        assert marker(owner) == ("approved" if deferred else "original")
        assert marker(sibling) == "original"
    finally:
        if agent is not None:
            agent.close()
        operator.close()
        gateway.stop()
