"""Process Security Engine decisions are visible through real `capsem logs`.

This is intentionally a real CLI/service/VM test: the lower unit tests prove
each boundary in isolation, while this catches the full propagation path:
runtime rule install -> capsem-process eval -> process.log -> /logs -> CLI.
"""

import time
import uuid

import pytest

pytestmark = pytest.mark.e2e


def _name(prefix: str) -> str:
    return f"{prefix}-{uuid.uuid4().hex[:8]}"


def _logs_until(service, vm: str, needles: list[str], *, timeout: float = 10.0) -> str:
    deadline = time.time() + timeout
    last_output = ""
    while time.time() < deadline:
        logs = service.cli_ok("logs", "--tail", "120", vm, timeout=30)
        last_output = logs.stdout
        if all(needle in last_output for needle in needles):
            return last_output
        time.sleep(0.25)
    return last_output


def test_runtime_process_block_is_visible_in_capsem_logs(service):
    vm = _name("plog")
    rule_id = f"runtime.block-shell-e2e.{uuid.uuid4().hex[:8]}"
    condition = (
        "process.activity.operation == 'exec' "
        "&& process.activity.command_class == 'shell'"
    )
    reason = "shell exec blocked by e2e"

    service.cli_ok("create", vm, timeout=180)
    try:
        assert service.wait_exec_ready(vm, timeout=180), f"VM {vm} never exec-ready"
        service.cli_ok(
            "enforcement",
            "install",
            rule_id,
            "--condition",
            condition,
            "--decision",
            "block",
            "--reason",
            reason,
            "--json",
            timeout=60,
        )

        blocked = service.cli("exec", vm, "bash -lc 'echo should-not-run'", timeout=60)
        combined = blocked.stdout + blocked.stderr
        assert blocked.returncode != 0, combined
        assert "process exec blocked" in combined
        assert rule_id in combined

        logs = _logs_until(
            service,
            vm,
            [
                "process_exec_security_decision",
                '"target":"security.process"',
                '"event_type":"process.exec"',
                '"final_action":"block"',
                f'"rule_id":"{rule_id}"',
                f'"reason":"{reason}"',
                f'"vm_id":"{vm}"',
                '"command_class":"shell"',
            ],
        )
        assert "process_exec_security_decision" in logs, logs
        assert '"target":"security.process"' in logs
        assert '"event_type":"process.exec"' in logs
        assert '"final_action":"block"' in logs
        assert f'"rule_id":"{rule_id}"' in logs
        assert f'"reason":"{reason}"' in logs
        assert f'"vm_id":"{vm}"' in logs
        assert '"command_class":"shell"' in logs
    finally:
        service.cli("enforcement", "delete", rule_id, timeout=60)
        service.cli("delete", vm, timeout=120)
