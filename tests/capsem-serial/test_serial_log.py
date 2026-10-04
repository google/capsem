"""Verify serial console logs are captured from the VM."""

import pytest

pytestmark = pytest.mark.serial


class TestSerialLog:

    def test_logs_endpoint_returns_data(self, serial_env):
        """GET /vms/{id}/logs returns non-empty content."""
        client, name = serial_env
        resp = client.get(f"/vms/{name}/logs")
        assert resp is not None, "Logs endpoint returned None"
        logs = resp.get("logs", "")
        assert len(logs) > 0, "Expected non-empty serial console logs"

    def test_logs_contain_kernel_output(self, serial_env):
        """Serial logs contain Linux kernel boot messages."""
        client, name = serial_env
        resp = client.get(f"/vms/{name}/logs")
        logs = resp.get("logs", "") if resp else ""
        # Kernel boot should mention Linux, console, or capsem
        assert any(kw in logs for kw in ["Linux", "console", "capsem", "init"]), (
            f"Expected kernel boot output in logs, got first 200 chars: {logs[:200]}"
        )

    def test_logs_available_before_delete(self, serial_env):
        """Logs can be retrieved while VM is running (before delete)."""
        client, name = serial_env
        # Retrieve logs twice to ensure they're consistently available
        resp1 = client.get(f"/vms/{name}/logs")
        resp2 = client.get(f"/vms/{name}/logs")
        assert resp1 is not None
        assert resp2 is not None
        logs1 = resp1.get("logs", "")
        logs2 = resp2.get("logs", "")
        # Second call should have at least as much content
        assert len(logs2) >= len(logs1)


# Every byte capsem-init writes to the serial console is on the exec-ready
# path. Since the emulated 16550 raises its transmit interrupt (92295e59b),
# userspace console writes block until each byte has left through a port-I/O
# exit: about 95us a byte on nested KVM, so the 4.3 KiB this console carried
# before the shell banner at the time cost ~0.4s of every boot and pushed exec
# latency past its 2s gate. Progress belongs in the host-preserved boot log;
# the console keeps failures.
BOOT_CONSOLE_BUDGET_BYTES = 1024
# The first thing the guest shell prints: everything before it is boot.
AGENT_START_MARKER = "welcome to"


class TestBootConsoleVolume:

    def test_boot_console_stays_within_its_byte_budget(self, serial_env):
        client, name = serial_env
        logs = client.get(f"/vms/{name}/logs").get("logs", "")
        assert AGENT_START_MARKER in logs, logs[:500]
        boot = logs.split(AGENT_START_MARKER, 1)[0]
        assert len(boot.encode()) <= BOOT_CONSOLE_BUDGET_BYTES, (
            f"{len(boot.encode())} console bytes before the agent started "
            f"(budget {BOOT_CONSOLE_BUDGET_BYTES}):\n{boot}"
        )

    def test_boot_console_carries_each_init_line_once(self, serial_env):
        """The kmsg copy of an init line is for dmesg, not a second console write."""
        client, name = serial_env
        logs = client.get(f"/vms/{name}/logs").get("logs", "")
        boot = logs.split(AGENT_START_MARKER, 1)[0]
        lines = [line for line in boot.splitlines() if line.startswith("[capsem-init] ")]
        repeated = sorted({line for line in lines if lines.count(line) > 1})
        assert not repeated, repeated

    def test_boot_reports_no_ownership_failures(self, serial_env):
        client, name = serial_env
        logs = client.get(f"/vms/{name}/logs").get("logs", "")
        assert "can't preserve ownership" not in logs
        assert "failed to preserve ownership" not in logs
