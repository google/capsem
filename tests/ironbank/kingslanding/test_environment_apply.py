"""An explicit operator application targets only the reviewed sandbox generations."""

import uuid
from contextlib import closing

import pytest
from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.gateway import TcpHttpClient
from helpers.service import ServiceInstance, exec_output_text, vm_name
from helpers.service_gateway import ServiceGateway

from tests.ironbank.kingslanding.test_environment_approval import delegate
from tests.ironbank.kingslanding.test_environment_startup import create, request, save
from tests.ironbank.kingslanding.test_run import service

__all__ = ["service"]
pytestmark = pytest.mark.integration


def test_apply_keeps_unselected_sandboxes_pinned_and_reports_each_acknowledgement(service):
    specification = {"name": "apply-operator", "variables": {"WORKBENCH_MODE": "original"}}
    original = save(service, specification)
    first, second, sibling = [create(service, original, vm_name(name)) for name in ("apply-first", "apply-second", "apply-sibling")]
    gateway = ServiceGateway(service)
    gateway.start()
    try:
        with closing(TcpHttpClient(gateway.base_url, gateway.token)) as operator:
            def binding(vm):
                return operator.get(f"/vms/{vm}/info")["environment"]

            def observed(vm):
                value = binding(vm)
                return {"session_id": vm, "generation": value["owner_generation"], "base": value["desired"]}

            def restart(vm):
                request(service, "POST", f"/vms/{vm}/stop", {})
                request(service, "POST", f"/vms/{vm}/start", {})

            def marker(vm):
                result = request(service, "POST", f"/vms/{vm}/exec", {"command": 'printf "%s" "$WORKBENCH_MODE"'})
                assert result["exit_code"] == 0, result
                return exec_output_text(result)

            root = f"/environments/{original['environment_id']}"
            revised = operator.post(root + "/changes", {
                "request_id": str(uuid.uuid4()), "base_revision_id": original["revision_id"],
                "specification": {
                    **specification,
                    "variables": {"WORKBENCH_MODE": "approved"},
                    "access": {"custom": {"preview_fixture": {
                        "name": "Preview fixture", "action": "block", "match": 'http.host == "preview.example"',
                    }}},
                },
                "reason": "prepare an exact saved revision",
            })["result"]
            stale = observed(second)
            restart(second)
            targets = [observed(first), observed(second)]
            assert stale["generation"] != targets[1]["generation"]
            sibling_binding = binding(sibling)
            first_binding = binding(first)
            path = root + "/apply"
            body = {
                "request_id": str(uuid.uuid4()), "base_revision_id": revised["revision_id"],
                "revision_id": revised["revision_id"], "targets": targets,
                "reason": "apply the reviewed revision to two current owners",
            }
            stale_body = {**body, "request_id": str(uuid.uuid4()), "targets": [targets[0], stale]}
            status, response = operator.call_json("POST", path, stale_body)
            assert status == 409, response
            assert binding(first) == first_binding
            preview_body = {key: body[key] for key in ("base_revision_id", "revision_id", "targets")}
            preview_path = path + "/preview"
            status, response = operator.call_json("POST", preview_path, {**preview_body, "targets": [stale]})
            assert status == 409, response
            with closing(delegate(operator, gateway, first, targets[0]["generation"])) as agent:
                assert agent.call_json("POST", path, body)[0] == 403
                assert agent.call_json("POST", preview_path, preview_body)[0] == 403

            preview = operator.post(preview_path, preview_body)
            assert preview["targets"] == [
                {"target": target, "boot": original, "live_policy": original,
                 "live_changes": ["access_rules"], "next_start_changes": ["variables"]}
                for target in targets
            ]
            assert binding(first) == first_binding
            assert binding(sibling) == sibling_binding
            assert operator.call_json("GET", f"/requests/{body['request_id']}/info")[0] == 404

            status, receipt = operator.call_json("POST", path, body)
            assert status == 200, receipt
            assert receipt["decision"] == "approved" and receipt["application"] == "pending"
            assert receipt["result"] == revised
            assert operator.call_json("POST", path, body) == (200, receipt)
            assert operator.call_json("POST", path, {**body, "targets": [targets[0]]})[0] == 409
            altered = [{**targets[0], "base": revised}, targets[1]]
            assert operator.call_json("POST", path, {**body, "targets": altered})[0] == 409
            assert binding(sibling) == sibling_binding
            for vm in (first, second):
                state = binding(vm)
                assert state["boot"] == original and state["live_policy"] == state["desired"] == revised
                assert state["pending_restart"]
                assert marker(vm) == "original"

            request_path = f"/requests/{body['request_id']}/info"
            initial = operator.get(request_path)
            assert initial["summary"]["kind"] == "environment_apply"
            assert all(row["state"] == "pending" and row["composition"]["pending_restart"] for row in initial["applications"])
            restart(first)
            partial = operator.get(request_path)
            assert partial["summary"]["application"] == "pending"
            statuses = {row["target"]["session_id"]: row["state"] for row in partial["applications"]}
            assert statuses == {first: "applied", second: "pending"}
            rollback_preview = operator.post(preview_path, {
                **preview_body, "revision_id": original["revision_id"], "targets": [observed(first), observed(second)],
            })["targets"]
            assert [(item["boot"], item["live_policy"], item["live_changes"], item["next_start_changes"])
                    for item in rollback_preview] == [
                (revised, revised, ["access_rules"], ["variables"]),
                (original, revised, ["access_rules"], []),
            ]
            restart(second)
            complete = operator.get(request_path)
            assert complete["summary"]["application"] == "applied"
            assert all(row["state"] == "applied" for row in complete["applications"])
            assert marker(first) == marker(second) == "approved"
            assert binding(sibling) == sibling_binding and marker(sibling) == "original"
    finally:
        gateway.stop()


def test_corporate_reload_preserves_each_acknowledged_environment_policy(monkeypatch, tmp_path):
    policy = tmp_path / "reload-corp.toml"
    baseline = 'refresh_policy = "24h"\n'
    policy.write_text(baseline)
    monkeypatch.setenv("CAPSEM_CORP_CONFIG", str(policy))
    service = ServiceInstance()
    gateway = ServiceGateway(service)
    shared = "CORPORATE_REFRESH_FIXTURE"
    selected_only = "ENVIRONMENT_REFRESH_FIXTURE"
    try:
        service.start()
        gateway.start()
        access = {"permit_fixture": {
            "name": "Allow the corporate refresh fixture",
            "action": "allow",
            "match": f'process.command.contains("{shared}")',
        }}
        specification = {
            "name": "corporate-reload-scope",
            "variables": {"RELOAD_REVISION": "original"},
            "access": {"custom": access},
        }
        original = save(service, specification)
        target = create(service, original, vm_name("reload-target"))
        sibling = create(service, original, vm_name("reload-sibling"))
        with closing(TcpHttpClient(gateway.base_url, gateway.token)) as operator:
            def binding(vm):
                return operator.get(f"/vms/{vm}/info")["environment"]

            def probe(vm, command):
                result = operator.post(f"/vms/{vm}/exec", {"command": command, "target": "vm"})
                return result["exit_code"] == 0, exec_output_text(result)

            def assert_probes(corporate_allows):
                for vm in (target, sibling):
                    allowed, text = probe(vm, f"printf %s {shared}")
                    assert allowed is corporate_allows
                    if allowed:
                        assert text == shared
                assert probe(target, f"printf %s {selected_only}")[0] is False
                assert probe(sibling, f"printf %s {selected_only}") == (True, selected_only)
                for vm in (target, sibling):
                    assert probe(vm, 'printf "%s" "$RELOAD_REVISION"') == (True, "original")

            root = f"/environments/{original['environment_id']}"
            revised = operator.post(root + "/changes", {
                "request_id": str(uuid.uuid4()), "base_revision_id": original["revision_id"],
                "reason": "Defer variables but apply this rule now",
                "specification": {
                    **specification,
                    "variables": {"RELOAD_REVISION": "next"},
                    "access": {"custom": {**access, "block_selected": {
                        "name": "Block only the selected sandbox's fixture",
                        "action": "block",
                        "match": f'process.command.contains("{selected_only}")',
                    }}},
                },
            })["result"]
            receipt = operator.post(root + "/apply", {
                "request_id": str(uuid.uuid4()), "base_revision_id": revised["revision_id"],
                "revision_id": revised["revision_id"], "reason": "Apply to one running sandbox",
                "targets": [{"session_id": target, "generation": binding(target)["owner_generation"],
                             "base": original}],
            })
            assert receipt["application"] == "pending", receipt
            acknowledged, untouched = binding(target), binding(sibling)
            assert acknowledged["boot"] == original
            assert acknowledged["live_policy"] == acknowledged["desired"] == revised
            assert_probes(corporate_allows=True)

            policy.write_text(baseline + f'''[corp.rules.reload_fixture]
name = "reload_fixture"
action = "block"
priority = -100
match = 'process.command.contains("{shared}")'
''')
            assert operator.post("/corp/reload", {}) == {"success": True, "reloaded": 2}
            assert_probes(corporate_allows=False)
            for vm, before in ((target, acknowledged), (sibling, untouched)):
                narrowed = binding(vm)
                assert narrowed["policy_digest"] != before["policy_digest"]
                assert narrowed == {**before, "policy_digest": narrowed["policy_digest"]}

            policy.write_text(baseline)
            assert operator.post("/corp/reload", {}) == {"success": True, "reloaded": 2}
            assert_probes(corporate_allows=True)
            assert binding(target) == acknowledged and binding(sibling) == untouched
    finally:
        gateway.stop()
        service.stop()


def test_apply_can_retry_only_the_sandbox_whose_selection_write_failed(service):
    original = save(service, {"name": "apply-retry"})
    ephemeral = request(service, "POST", "/vms/create", {
        "environment": original, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS,
    })["id"]
    persistent = create(service, original, vm_name("apply-retry-persistent"))
    gateway = ServiceGateway(service)
    gateway.start()
    try:
        with closing(TcpHttpClient(gateway.base_url, gateway.token)) as operator:
            def selection(vm):
                info = operator.get(f"/vms/{vm}/info")
                environment = info["environment"]
                return {"session_id": vm, "generation": environment["owner_generation"], "base": environment["desired"]}

            root = f"/environments/{original['environment_id']}"
            revision = operator.post(root + "/changes", {
                "request_id": str(uuid.uuid4()), "base_revision_id": original["revision_id"],
                "specification": {"name": "apply-retry", "description": "reviewed"},
                "reason": "prepare the same revision for two different lifetimes",
            })["result"]
            path = root + "/apply"
            body = {
                "request_id": str(uuid.uuid4()), "base_revision_id": revision["revision_id"],
                "revision_id": revision["revision_id"], "targets": [selection(ephemeral), selection(persistent)],
                "reason": "apply the reviewed revision to both sandbox generations",
            }
            registry = service.tmp_dir / "persistent_registry.json"
            backup = registry.with_suffix(".apply-test-backup")
            registry.rename(backup)
            try:
                registry.mkdir()
                status, failed = operator.call_json("POST", path, body)
                assert status == 200 and failed["application"] == "failed", failed
            finally:
                if registry.is_dir():
                    registry.rmdir()
                backup.rename(registry)

            audit = operator.get(f"/requests/{body['request_id']}/info")
            outcomes = {item["target"]["session_id"]: item for item in audit["applications"]}
            assert outcomes[ephemeral]["state"] == "applied"
            assert outcomes[persistent]["state"] == "failed"
            assert "selection could not be saved" in outcomes[persistent]["failure_reason"]
            assert selection(ephemeral)["base"] == revision
            assert selection(persistent)["base"] == original
            retry = {**body, "request_id": str(uuid.uuid4()), "targets": [selection(persistent)]}
            status, receipt = operator.call_json("POST", path, retry)
            assert status == 200 and receipt["application"] == "applied", receipt
            assert selection(persistent)["base"] == revision
            assert operator.get(f"/requests/{body['request_id']}/info")["applications"] == audit["applications"]
    finally:
        gateway.stop()
