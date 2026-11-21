"""An operator reviews an exact environment request through the built CLI."""

import json
import subprocess
import uuid
from contextlib import closing

import pytest
from helpers.gateway import TcpHttpClient
from helpers.service import exec_output_text, vm_name
from helpers.service_gateway import ServiceGateway

from tests.ironbank.kingslanding.test_environment_approval import delegate
from tests.ironbank.kingslanding.test_environment_startup import create, request, save
from tests.ironbank.kingslanding.test_run import cli, environment, service

__all__ = ["service"]
pytestmark = pytest.mark.integration


def test_cli_reviews_the_original_proposal_and_reports_deferred_application(service):
    initial = {"name": "cli-approval", "variables": {"REVIEWED_SETTING": "original"}}
    base = save(service, initial)
    vm = create(service, base, vm_name("cli-approval"))
    vm_info = f"/vms/{vm}/info"
    original = request(service, "GET", vm_info)["environment"]

    gateway = ServiceGateway(service)
    gateway.start()
    try:
        cli_env = {
            **environment(service),
            "CAPSEM_GATEWAY_URL": gateway.base_url,
            "CAPSEM_GATEWAY_TOKEN": gateway.token,
        }

        def run_cli(*args, expected=0):
            result = subprocess.run(
                cli(service, *args),
                env=cli_env,
                capture_output=True,
                check=False,
                timeout=30,
            )
            assert result.returncode == expected, (result.stdout, result.stderr)
            return result.stdout.decode() if expected == 0 else result.stderr.decode()

        def review(*args, expected=0):
            output = run_cli("requests", *args, expected=expected)
            return json.loads(output) if expected == 0 else output

        def marker():
            result = request(
                service,
                "POST",
                f"/vms/{vm}/exec",
                {
                    "command": 'printf "%s" "$REVIEWED_SETTING"',
                },
            )
            assert result["exit_code"] == 0, result
            return exec_output_text(result)

        with (
            closing(TcpHttpClient(gateway.base_url, gateway.token)) as operator,
            closing(
                delegate(operator, gateway, vm, original["owner_generation"])
            ) as agent,
        ):
            item_id = str(uuid.uuid4())
            status, queued = agent.call_json(
                "POST",
                f"/environments/{base['environment_id']}/changes",
                {
                    "request_id": item_id,
                    "base_revision_id": base["revision_id"],
                    "specification": {
                        **initial,
                        "variables": {"REVIEWED_SETTING": "approved"},
                    },
                    "reason": "change one variable at next start",
                },
            )
            assert status == 200, queued

        listing = review("list")
        assert [item["request_id"] for item in listing["requests"]] == [item_id]
        detail = review("info", item_id)
        assert detail["summary"] == listing["requests"][0]
        assert detail["base"]["specification"]["variables"] == {
            "REVIEWED_SETTING": "original"
        }
        assert detail["proposed"]["specification"]["variables"] == {
            "REVIEWED_SETTING": "approved"
        }
        observation = detail["observed_revision"]
        assert queued["observed_revision"] == observation

        command = (
            "resolve",
            item_id,
            "approve",
            "--reason",
            "apply the reviewed value",
        )
        assert "--observed-revision" in review(*command, expected=2)
        wrong_observation = "0" * 12 if observation != "0" * 12 else "1" * 12
        assert "409" in review(
            *command, "--observed-revision", wrong_observation, expected=1
        )
        assert request(service, "GET", vm_info)["environment"] == original

        receipt = review(*command, "--observed-revision", observation)
        assert receipt["request_id"] == item_id
        assert receipt["decision"] == "approved" and receipt["application"] == "pending"
        assert receipt["result"]["environment_id"] != base["environment_id"]
        assert (
            request(service, "GET", vm_info)["environment"]["desired"]
            == receipt["result"]
        )
        assert review("list")["requests"] == []
        inspected = run_cli("info", vm)
        fork = receipt["result"]
        base_revision = f"{base['environment_id']}@{base['revision_id']}"
        fork_revision = f"{fork['environment_id']}@{fork['revision_id']}"
        assert f"Boot environment: {base_revision}" in inspected
        assert f"Live policy revision: {fork_revision}" in inspected
        assert f"Desired environment: {fork_revision}" in inspected
        assert "Workload changes pending restart" in inspected
        assert marker() == "original"
        request(service, "POST", f"/vms/{vm}/stop", {})
        request(service, "POST", f"/vms/{vm}/start", {})
        assert marker() == "approved"
        restarted = run_cli("info", vm)
        assert f"Boot environment: {fork_revision}" in restarted
        assert "Workload changes pending restart" not in restarted
        applied = review("info", item_id)["summary"]
        assert (
            applied["application"] == "applied"
            and applied["result"] == receipt["result"]
        )
        assert applied in review("list", "--all")["requests"]
    finally:
        gateway.stop()
