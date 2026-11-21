"""An environment's rule view comes from the rule set used at launch."""

import secrets
import uuid
from contextlib import closing

import pytest
from helpers.gateway import TcpHttpClient
from helpers.service import ServiceInstance, exec_output_text, vm_name, wait_exec_ready
from helpers.service_gateway import ServiceGateway

from tests.ironbank.kingslanding.test_environment_startup import create, save

pytestmark = pytest.mark.integration


def test_current_environment_rules_show_corporate_precedence_and_separate_observations(monkeypatch, tmp_path):
    policy = tmp_path / "corp.toml"
    policy.write_text('''refresh_policy = "24h"
[corp.rules.restrict_fixture]
name = "corporate_fixture_rule"
action = "block"
priority = -100
match = 'process.command.contains("RULE_VIEW_FIXTURE")'
''')
    monkeypatch.setenv("CAPSEM_CORP_CONFIG", str(policy))
    service = ServiceInstance()
    gateway = ServiceGateway(service)
    condition = 'process.command.contains("RULE_VIEW_FIXTURE")'
    try:
        service.start()
        saved = save(service, {
            "name": "rule-preview",
            "access": {"custom": {"allow_fixture": {
                "name": "Environment allowance", "match": condition, "action": "allow",
            }}},
            "detection": {"custom": {"observe_fixture": {
                "name": "Environment observation", "match": condition, "level": "high",
            }}},
        })
        other = save(service, {"name": "other-rule-preview"})
        gateway.start()
        route = f"/environments/{saved['environment_id']}/rules/list"
        exact = f"{route}?revision_id={saved['revision_id']}"
        with closing(TcpHttpClient(gateway.base_url, gateway.token)) as operator:
            status, result = operator.call_json("GET", exact)
            assert status == 200, result
            assert result["revision"] == saved
            matching = [entry for entry in result["access"] if entry["condition"] == condition]
            assert [(rule["source"], rule["action"], rule["locked"]) for rule in matching] == [
                ("corporate", "block", True), ("environment", "allow", False),
            ]
            assert matching[0]["priority"] == -100 < matching[1]["priority"]
            assert matching[0]["editable_key"] is None and matching[1]["editable_key"] == "allow_fixture"
            assert result["detection"] == [{
                "id": "environment.detection.observe_fixture", "name": "Environment observation",
                "condition": condition, "level": "high", "enabled": True, "source": "environment",
                "editable_key": "observe_fixture",
            }]
            assert operator.call_json("GET", f"{route}?revision_id={other['revision_id']}")[0] == 409
            secret = secrets.token_hex(32)
            status, grant = operator.call_json("POST", "/agent-access", {
                "request_id": str(uuid.uuid4()), "ownership_token": secret,
                "lease_seconds": 300,
                "scope": {"operations": ["inspect"], "environment_ids": [saved["environment_id"]]},
            })
            assert status == 200, grant
            with closing(TcpHttpClient(gateway.base_url, f"{grant['access']['id']}.{secret}")) as agent:
                assert agent.call_json("GET", exact) == (200, result)
                assert agent.call_json("GET", f"/environments/{other['environment_id']}/rules/list")[0] == 403
            verify_launch_uses_the_compiled_precedence(service, operator, saved)
    finally:
        gateway.stop()
        service.stop()


def verify_launch_uses_the_compiled_precedence(service, operator, reference):
    vm = create(service, reference, vm_name("effective-rules"))
    route = f"/vms/{vm}"
    try:
        assert wait_exec_ready(service.client(), vm)
        assert operator.get(route + "/info")["environment"]["boot"] == reference
        command = {"command": "touch /tmp/preview-rule-effect; : RULE_VIEW_FIXTURE", "target": "vm", "timeout_secs": 10}
        denied = operator.post(route + "/exec", command)
        assert denied["exit_code"] != 0, denied
        command["command"] = "test ! -e /tmp/preview-rule-effect && printf ready"
        unaffected = operator.post(route + "/exec", command)
        assert unaffected["exit_code"] == 0 and exec_output_text(unaffected) == "ready"
    finally:
        service.client().delete(route + "/delete", timeout=60)
