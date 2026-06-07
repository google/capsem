"""Serial console logs endpoint tests."""

import pytest

pytestmark = pytest.mark.integration


class TestLogs:

    def test_logs_nonempty(self, ready_vm):
        client, name = ready_vm
        resp = client.get(f"/logs/{name}")
        assert resp is not None
        logs = resp.get("logs", "")
        assert len(logs) > 0, "Expected non-empty serial console logs"

    def test_logs_contain_boot_output(self, ready_vm):
        """Serial logs should contain kernel or init output."""
        client, name = ready_vm
        resp = client.get(f"/logs/{name}")
        logs = resp.get("logs", "")
        assert "Linux" in logs or "console" in logs or "capsem" in logs.lower(), (
            f"Expected boot output in logs, got: {logs[:200]}"
        )

    def test_logs_nonexistent_vm(self, service_env):
        client = service_env.client()
        resp = client.get("/logs/ghost-vm-404")
        assert resp is None or "error" in str(resp).lower() or "not found" in str(resp).lower()
