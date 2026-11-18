"""A saved revision stays bound across real VM cold starts and forks."""

from __future__ import annotations

import copy
import uuid

import pytest
from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.service import ServiceInstance, exec_output_text, vm_name

pytestmark = pytest.mark.integration


def _request(client, method, path, body=None):
    status, result = client.call_json(method, path, body, timeout=60)
    assert status == 200, (method, path, status, result)
    return result


def _environment(client, vm, reference):
    info = _request(client, "GET", f"/vms/{vm}/info")
    value = info["environment"]
    assert set(value) == {
        "boot", "live_policy", "desired", "owner_generation", "policy_digest", "pending_restart",
    }
    assert value["boot"] == reference
    assert value["live_policy"] == reference
    assert value["desired"] == reference
    assert str(uuid.UUID(value["owner_generation"])) == value["owner_generation"]
    assert value["policy_digest"].startswith("blake3:")
    assert len(value["policy_digest"]) == 71
    assert value["pending_restart"] is False
    execution = _request(client, "POST", f"/vms/{vm}/exec", {"command": 'printf "%s" "$BOUND_REVISION"'})
    assert execution["exit_code"] == 0
    assert exec_output_text(execution) == "original"
    return value["owner_generation"]


def test_environment_revision_survives_shared_edits_resume_and_fork():
    service = ServiceInstance(isolated_corp_config=True)
    service.start()
    client = service.client()
    created = []
    try:
        specification = {
            "name": "launch-proof",
            "variables": {"BOUND_REVISION": "original"},
            "access": {"custom": {"scoped": {
                "name": "Scoped network", "action": "block", "match": 'http.host == "blocked.example"',
            }}},
        }
        saved = _request(client, "POST", "/environments/create", {
            "request_id": str(uuid.uuid4()), "specification": specification, "reason": "native launch proof",
        })
        assert saved["decision"] == "approved"
        assert saved["application"] == "applied"
        reference = saved["result"]
        selected = _request(client, "POST", "/vms/create", {
            "name": vm_name("environment"), "environment": reference,
            "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS,
        })
        vm = selected["id"]
        created.append(vm)
        first_generation = _environment(client, vm, reference)
        changed = copy.deepcopy(specification)
        changed["variables"]["BOUND_REVISION"] = "new-shared-head"
        edited = _request(client, "POST", f'/environments/{reference["environment_id"]}/changes', {
            "request_id": str(uuid.uuid4()), "base_revision_id": reference["revision_id"],
            "specification": changed, "reason": "future creations only",
        })
        assert edited["result"]["revision_id"] != reference["revision_id"]
        assert _environment(client, vm, reference) == first_generation
        _request(client, "POST", "/corp/reload", {})
        assert _environment(client, vm, reference) == first_generation
        _request(client, "POST", f"/vms/{vm}/stop", {})
        stopped = _request(client, "GET", f"/vms/{vm}/info")["environment"]
        assert stopped["desired"] == reference
        assert stopped["boot"] is None
        assert stopped["live_policy"] is None
        assert stopped["owner_generation"] is None
        assert stopped["policy_digest"] is None
        assert stopped["pending_restart"] is None
        _request(client, "POST", f"/vms/{vm}/resume", {})
        resumed_generation = _environment(client, vm, reference)
        assert resumed_generation != first_generation
        fork = _request(client, "POST", f"/vms/{vm}/fork", {"name": vm_name("env-fork")})
        created.append(fork["id"])
        _request(client, "POST", f'/vms/{fork["id"]}/resume', {})
        fork_generation = _environment(client, fork["id"], reference)
        assert fork_generation not in {first_generation, resumed_generation}
        assert _environment(client, vm, reference) == resumed_generation
    finally:
        try:
            for vm in reversed(created):
                _request(client, "DELETE", f"/vms/{vm}/delete")
        finally:
            service.stop()
