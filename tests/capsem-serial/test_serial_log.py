"""Verify serial console logs are captured from the VM."""

import pytest

pytestmark = pytest.mark.serial


class TestSerialLog:

    def test_logs_endpoint_returns_data(self, serial_env):
        """GET /logs/{id} returns non-empty content."""
        client, name = serial_env
        resp = client.get(f"/logs/{name}")
        assert resp is not None, "Logs endpoint returned None"
        logs = resp.get("logs", "")
        assert len(logs) > 0, "Expected non-empty serial console logs"

    def test_logs_contain_kernel_output(self, serial_env):
        """Serial logs contain Linux kernel boot messages."""
        client, name = serial_env
        resp = client.get(f"/logs/{name}")
        logs = resp.get("logs", "") if resp else ""
        # Kernel boot should mention Linux, console, or capsem
        assert any(kw in logs for kw in ["Linux", "console", "capsem", "init"]), (
            f"Expected kernel boot output in logs, got first 200 chars: {logs[:200]}"
        )

    def test_logs_available_before_delete(self, serial_env):
        """Logs can be retrieved while VM is running (before delete)."""
        client, name = serial_env
        # Retrieve logs twice to ensure they're consistently available
        resp1 = client.get(f"/logs/{name}")
        resp2 = client.get(f"/logs/{name}")
        assert resp1 is not None
        assert resp2 is not None
        logs1 = resp1.get("logs", "")
        logs2 = resp2.get("logs", "")
        # Second call should have at least as much content
        assert len(logs2) >= len(logs1)
