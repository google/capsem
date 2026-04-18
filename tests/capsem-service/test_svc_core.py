"""Core no-state service endpoints: /version, /stats, /service-logs, /reload-config."""

import pytest

pytestmark = pytest.mark.integration


class TestVersion:

    def test_version_returns_string(self, client):
        resp = client.get("/version")
        assert resp is not None
        version = resp.get("version")
        assert isinstance(version, str) and version, f"empty version: {resp}"
        # Version follows "1.0.<timestamp>" convention from workspace package.
        assert version.startswith("1."), f"unexpected version: {version}"


class TestStats:

    def test_stats_shape(self, client):
        """/stats returns the top-level StatsResponse shape whether or not sessions exist."""
        resp = client.get("/stats")
        assert resp is not None
        for key in ("global", "sessions", "top_providers", "top_tools", "top_mcp_tools"):
            assert key in resp, f"missing '{key}' in /stats response: {list(resp.keys())}"
        assert isinstance(resp["sessions"], list)
        assert isinstance(resp["top_providers"], list)
        assert isinstance(resp["top_tools"], list)
        assert isinstance(resp["top_mcp_tools"], list)


class TestServiceLogs:

    def test_service_logs_present(self, client):
        """/service-logs returns the tail of the service's own log file as plain text."""
        # Trigger some recent activity so the log has content.
        client.get("/list")
        text = client.get_text("/service-logs")
        assert isinstance(text, str) and text, "service-logs returned empty"
        assert len(text) > 10, f"service-logs implausibly short: {text!r}"
        # Service log lines are JSON-structured; expect at least one `"target":"capsem_service"` entry.
        assert "capsem_service" in text, (
            f"no capsem_service target lines in service-logs output: {text[:300]!r}"
        )


class TestReloadConfig:

    def test_reload_config_no_instances(self, client):
        """/reload-config succeeds with instances: 0 when no VMs are running."""
        # Make sure no VMs are running first.
        client.post("/purge", {"all": True})

        resp = client.post("/reload-config", {})
        assert resp is not None, "reload-config returned no body"
        assert resp.get("success") is True, f"reload-config failed: {resp}"
        assert resp.get("reloaded") == 0, (
            f"expected 0 reloaded, got {resp.get('reloaded')}: {resp}"
        )
