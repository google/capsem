"""Delegated changes wait for operator review under the original VM identity."""

import secrets
import uuid

import pytest
from helpers.gateway import TcpHttpClient
from helpers.service import vm_name
from helpers.service_gateway import ServiceGateway

from tests.ironbank.kingslanding.test_environment_startup import create, request, save
from tests.ironbank.kingslanding.test_run import service

__all__ = ["service"]
pytestmark = pytest.mark.integration


def test_gateway_submission_preserves_the_committed_head_and_running_sibling(service):
    base = save(service, {"name": "proposals", "variables": {"EDITOR": "vim"}})
    owner = create(service, base, vm_name("proposer"))
    sibling = create(service, base, vm_name("sibling"))
    targets = {}
    for target in (owner, sibling):
        environment = request(service, "GET", f"/vms/{target}/info")["environment"]
        targets[target] = {
            "session_id": target,
            "owner_generation": environment["owner_generation"],
        }
        assert environment["boot"] == base

    gateway = ServiceGateway(service)
    gateway.start()
    operator = TcpHttpClient(gateway.base_url, gateway.token)

    def delegate(sessions):
        secret = secrets.token_hex(32)
        status, grant = operator.call_json(
            "POST",
            "/agent-access",
            {
                "request_id": str(uuid.uuid4()),
                "ownership_token": secret,
                "lease_seconds": 300,
                "scope": {"operations": ["propose"], "sessions": sessions},
            },
        )
        assert status == 200, grant
        return TcpHttpClient(gateway.base_url, f"{grant['access']['id']}.{secret}")

    agent = None
    ambiguous = None
    try:
        agent = delegate([targets[owner]])
        ambiguous = delegate(list(targets.values()))
        path = f"/environments/{base['environment_id']}/changes"
        payload = {
            "request_id": str(uuid.uuid4()),
            "base_revision_id": base["revision_id"],
            "specification": {"name": "proposals", "variables": {"EDITOR": "emacs"}},
            "reason": "propose an editor change for the current workspace",
        }
        assert ambiguous.call_json("POST", path, payload)[0] == 403
        status, proposal = agent.call_json("POST", path, payload)
        assert status == 200, proposal
        assert proposal == {
            "request_id": payload["request_id"],
            "environment_id": base["environment_id"],
            "decision": "pending",
            "application": "pending",
            "result": None,
            "application_error": None,
        }
        assert agent.call_json("POST", path, payload) == (200, proposal)
        head = operator.get(f"/environments/{base['environment_id']}/info")["document"]
        assert head["identity"]["revision_id"] == base["revision_id"]
        assert head["specification"]["variables"] == {"EDITOR": "vim"}
        for target in (owner, sibling):
            environment = operator.get(f"/vms/{target}/info")["environment"]
            assert environment["desired"] == environment["boot"] == base
            assert environment["owner_generation"] == targets[target]["owner_generation"]

        request(service, "POST", f"/vms/{owner}/stop", {})
        request(service, "POST", f"/vms/{owner}/start", {})
        assert operator.get(f"/vms/{owner}/info")["environment"]["owner_generation"] != (
            targets[owner]["owner_generation"]
        )
        assert agent.call_json(
            "POST", path, {**payload, "request_id": str(uuid.uuid4())}
        )[0] == 403
    finally:
        if agent is not None:
            agent.close()
        if ambiguous is not None:
            ambiguous.close()
        operator.close()
        gateway.stop()
