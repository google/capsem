"""An approved change adopts a fork in exactly one persistent VM."""

import json
import secrets
import time
import uuid
from contextlib import closing

import pytest
from helpers.gateway import TcpHttpClient
from helpers.service import exec_output_text, vm_name
from helpers.service_gateway import ServiceGateway
from websockets.sync.client import connect

from tests.fixtures.oci.registry import publish_catalog, registry
from tests.ironbank.kingslanding.test_environment_startup import create, request, save
from tests.ironbank.kingslanding.test_image_catalog import entry, use_catalog
from tests.ironbank.kingslanding.test_run import service

__all__ = ["service"]
pytestmark = pytest.mark.integration


def delegate(operator, gateway, vm, generation):
    secret = secrets.token_hex(32)
    status, issued = operator.call_json("POST", "/agent-access", {
        "request_id": str(uuid.uuid4()),
        "ownership_token": secret,
        "lease_seconds": 300,
        "scope": {"operations": ["propose", "cancel_own"], "sessions": [
            {"session_id": vm, "owner_generation": generation},
        ]},
    })
    assert status == 200, issued
    return TcpHttpClient(gateway.base_url, f"{issued['access']['id']}.{secret}")


def test_environment_events_follow_durable_queue_changes_without_replaying(service):
    specification = {"name": "event-review"}
    base = save(service, specification)
    vm = create(service, base, vm_name("event-review"))
    generation = request(service, "GET", f"/vms/{vm}/info")["environment"]["owner_generation"]
    gateway = ServiceGateway(service)
    gateway.start()
    operator = TcpHttpClient(gateway.base_url, gateway.token)
    agent = None
    try:
        agent = delegate(operator, gateway, vm, generation)
        request_id = str(uuid.uuid4())
        input = {
            "request_id": request_id, "base_revision_id": base["revision_id"],
            "specification": {**specification, "variables": {"EDITOR": "emacs"}},
            "reason": "review an editor change",
        }
        proposal = f"/environments/{base['environment_id']}/changes"
        review = f"/requests/{request_id}"
        with connect(gateway.base_url.replace("http:", "ws:", 1) + "/events",
                     additional_headers={"Authorization": f"Bearer {gateway.token}"},
                     open_timeout=5, close_timeout=2) as socket:
            def notice(timeout=5):
                deadline = time.monotonic() + timeout
                while deadline > time.monotonic():
                    message = json.loads(socket.recv(timeout=max(0, deadline - time.monotonic())))
                    if message["type"] == "environment-requests-changed":
                        return message["payload"]["id"]
                raise TimeoutError("no environment request change")

            status, queued = agent.call_json("POST", proposal, input)
            assert status == 200, queued
            assert notice() == request_id
            assert operator.get(review + "/info")["summary"]["decision"] == "pending"
            assert agent.call_json("POST", proposal, input) == (200, queued)
            with pytest.raises(TimeoutError):
                notice(0.3)
            cancel = {
                "observed_revision": queued["observed_revision"],
                "decision": "cancel", "reason": "withdraw this proposal",
            }
            status, receipt = agent.call_json("POST", review + "/resolve", cancel)
            assert status == 200, receipt
            assert notice() == request_id
            assert operator.get(review + "/info")["summary"]["decision"] == "cancelled"
            assert agent.call_json("POST", review + "/resolve", cancel) == (200, receipt)
            with pytest.raises(TimeoutError):
                notice(0.3)
    finally:
        if agent is not None:
            agent.close()
        operator.close()
        gateway.stop()


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
        agent = delegate(operator, gateway, owner, original["owner_generation"])
        proposal_path = f"/environments/{base['environment_id']}/changes"
        proposed = {
            **specification,
            "variables": {"CAPSEM_REVIEW_MARKER": "approved" if deferred else "original"},
            "packages": {"allows_package_manager": True, "registries": ["npm"]},
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
            assert detail["restart_required"] is deferred
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
        fork_rules = operator.get(f"/environments/{fork['environment_id']}/rules/list")["access"]
        package_rules = [rule for rule in fork_rules if rule["id"].startswith("environment.packages.")]
        assert [rule["id"] for rule in package_rules] == ["environment.packages.npm"]
        assert package_rules[0]["editable_key"] is None
        parent_rules = operator.get(f"/environments/{base['environment_id']}/rules/list")["access"]
        assert not any(rule["id"].startswith("environment.packages.") for rule in parent_rules)

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
        settled = operator.get(info_path)["summary"]
        assert settled["application"] == "applied"
        assert settled["result"] == fork
        assert operator.call_json("POST", path, approved) == (200, {**receipt, "application": "applied"})

        edit = {
            "request_id": str(uuid.uuid4()),
            "base_revision_id": fork["revision_id"],
            "specification": {**proposed, "description": "operator edit of the saved fork"},
            "reason": "update the saved fork for future launches",
        }
        fork_path = f"/environments/{fork['environment_id']}"
        status, edited = operator.call_json("POST", fork_path + "/changes", edit)
        assert status == 200, edited
        assert edited["application"] == "applied"
        assert edited["result"]["environment_id"] == fork["environment_id"]
        assert edited["result"]["revision_id"] != fork["revision_id"]
        assert operator.call_json("POST", fork_path + "/changes", edit) == (200, edited)
        saved_fork = operator.get(fork_path + "/info")["document"]
        assert saved_fork["identity"]["revision_id"] == edited["result"]["revision_id"]
        assert saved_fork["identity"]["parent_environment_id"] == base["environment_id"]
        assert saved_fork["identity"]["parent_revision_id"] == base["revision_id"]
        assert saved_fork["specification"]["description"] == "operator edit of the saved fork"
        assert operator.get(f"/environments/{base['environment_id']}/info")["document"] == shared
        assert operator.get(owner_path)["environment"] == restarted
        assert operator.get(sibling_path)["environment"] == sibling_binding
        future = create(service, edited["result"], vm_name("revised-fork"))
        future_binding = operator.get(f"/vms/{future}/info")["environment"]
        assert future_binding["boot"] == future_binding["live_policy"] == future_binding["desired"] == edited["result"]
        assert marker(future) == marker(owner)
    finally:
        if agent is not None:
            agent.close()
        operator.close()
        gateway.stop()


def test_operator_promotes_exact_fork_without_rebinding_running_vms(service):
    initial = {"name": "promoted-environment", "variables": {"WORKBENCH_MODE": "parent"}}
    parent = save(service, initial)
    owner = create(service, parent, vm_name("promote-owner"))
    sibling = create(service, parent, vm_name("promote-sibling"))
    owner_path = f"/vms/{owner}/info"
    sibling_path = f"/vms/{sibling}/info"
    parent_path = f"/environments/{parent['environment_id']}"
    generation = request(service, "GET", owner_path)["environment"]["owner_generation"]
    gateway = ServiceGateway(service)
    gateway.start()
    try:
        with (
            closing(TcpHttpClient(gateway.base_url, gateway.token)) as operator,
            closing(delegate(operator, gateway, owner, generation)) as agent,
        ):
            proposal_id = str(uuid.uuid4())
            proposed = {**initial, "variables": {"WORKBENCH_MODE": "fork"}}
            status, proposal = agent.call_json("POST", parent_path + "/changes", {
                "request_id": proposal_id, "base_revision_id": parent["revision_id"],
                "specification": proposed, "reason": "propose a sandbox-local variable",
            })
            assert status == 200, proposal
            status, approved = operator.call_json("POST", f"/requests/{proposal_id}/resolve", {
                "observed_revision": proposal["observed_revision"], "decision": "approve",
                "reason": "review and apply the sandbox-local proposal",
            })
            assert status == 200, approved
            fork = approved["result"]
            fork_path = f"/environments/{fork['environment_id']}"
            status, edit = operator.call_json("POST", fork_path + "/changes", {
                "request_id": str(uuid.uuid4()), "base_revision_id": fork["revision_id"],
                "specification": {**proposed, "description": "reviewed fork revision"},
                "reason": "make a reviewed revision of the fork",
            })
            assert status == 200, edit
            source = edit["result"]
            snapshot = operator.get(fork_path + "/info")["document"]["specification"]
            status, later_edit = operator.call_json("POST", fork_path + "/changes", {
                "request_id": str(uuid.uuid4()), "base_revision_id": source["revision_id"],
                "specification": {**proposed, "variables": {"WORKBENCH_MODE": "later"}},
                "reason": "advance the fork while preserving its reviewed revision",
            })
            assert status == 200 and later_edit["result"] != source, later_edit
            owner_binding = operator.get(owner_path)["environment"]
            sibling_binding = operator.get(sibling_path)["environment"]
            promote = {
                "request_id": str(uuid.uuid4()), "base_revision_id": parent["revision_id"],
                "source": source, "reason": "adopt this reviewed fork revision for future VMs",
            }
            promote_path = parent_path + "/promote"
            assert agent.call_json("POST", promote_path, promote)[0] == 403
            status, result = operator.call_json("POST", promote_path, promote)
            assert status == 200, result
            assert result["decision"] == "approved" and result["application"] == "applied"
            promoted = result["result"]
            assert promoted["environment_id"] == parent["environment_id"]
            assert promoted["revision_id"] != parent["revision_id"]
            assert agent.call_json("POST", promote_path, promote)[0] == 403
            assert operator.call_json("POST", promote_path, promote) == (200, result)
            assert operator.call_json("POST", promote_path, {**promote, "source": fork})[0] == 409
            stale = {**promote, "request_id": str(uuid.uuid4())}
            assert operator.call_json("POST", promote_path, stale)[0] == 409
            unrelated = {**stale, "base_revision_id": promoted["revision_id"], "source": parent}
            assert operator.call_json("POST", promote_path, unrelated)[0] == 409
            detail = operator.get(f"/requests/{promote['request_id']}/info")
            assert detail["summary"]["source"] == source
            assert detail["summary"]["kind"] == "environment_promotion"
            assert detail["proposed"]["specification"] == snapshot
            assert operator.get(parent_path + "/info")["document"]["specification"] == snapshot
            assert operator.get(owner_path)["environment"] == owner_binding
            assert operator.get(sibling_path)["environment"] == sibling_binding
            future = create(service, promoted, vm_name("promoted-vm"))
            value = request(service, "POST", f"/vms/{future}/exec", {
                "command": 'printf "%s" "$WORKBENCH_MODE"',
            })
            assert value["exit_code"] == 0 and exec_output_text(value) == "fork"
    finally:
        gateway.stop()


def test_workload_startup_failure_leaves_the_approved_request_pending(service, tmp_path):
    artifacts = {}
    with registry(tmp_path, artifacts=artifacts) as (image, certificate, _):
        publish_catalog(artifacts, "library/catalog", "stable", entry(image))
        use_catalog(service, f"{image.split('/', 1)[0]}/library/catalog:stable", certificate)
        specification = {"name": "failed-next-start", "image": {"reference": "redis"}}
        base = save(service, specification)
        vm = create(service, base, vm_name("failed-next-start"))
        generation = request(service, "GET", f"/vms/{vm}/info")["environment"]["owner_generation"]
        gateway = ServiceGateway(service)
        gateway.start()
        operator = TcpHttpClient(gateway.base_url, gateway.token)
        agent = None
        try:
            agent = delegate(operator, gateway, vm, generation)
            request_id = str(uuid.uuid4())
            status, queued = agent.call_json("POST", f"/environments/{base['environment_id']}/changes", {
                "request_id": request_id, "base_revision_id": base["revision_id"],
                "specification": {**specification, "startup": {"script": "exit 17", "timeout_seconds": 10}},
                "reason": "review a startup change with a known failure",
            })
            assert status == 200, queued
            approve = {
                "observed_revision": queued["observed_revision"],
                "decision": "approve", "reason": "exercise the next-start failure path",
            }
            resolve_path = f"/requests/{request_id}/resolve"
            status, receipt = operator.call_json("POST", resolve_path, approve)
            assert status == 200, receipt
            assert receipt["decision"] == "approved" and receipt["application"] == "pending"
            request(service, "POST", f"/vms/{vm}/stop", {})
            error = request(service, "POST", f"/vms/{vm}/start", {}, expected=500)
            assert "startup exit (17)" in error["error"], error
            assert request(service, "GET", f"/vms/{vm}/container")["state"] == "failed"
            assert operator.get(f"/requests/{request_id}/info")["summary"]["application"] == "pending"
            assert operator.call_json("POST", resolve_path, approve) == (200, receipt)
        finally:
            if agent is not None:
                agent.close()
            operator.close()
            gateway.stop()
