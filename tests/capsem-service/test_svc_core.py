"""Core no-state service endpoints: /version, /stats, /service-logs, profile reload."""

from pathlib import Path

import pytest
import tomllib
from log_streams import assert_service_log_evidence

pytestmark = pytest.mark.integration

PROJECT_ROOT = Path(__file__).resolve().parents[2]


class TestVersion:

    def test_version_returns_string(self, client):
        resp = client.get("/version")
        assert resp is not None
        version = resp.get("version")
        assert isinstance(version, str) and version, f"empty version: {resp}"
        # Compared against Cargo.toml rather than a prefix literal. The old
        # assertion was `startswith("1.")` for a "1.0.<timestamp>" convention
        # that no longer exists, so it failed the release rather than the
        # service. The real property is that the daemon reports the version it
        # was built from.
        workspace = tomllib.loads(
            (PROJECT_ROOT / "Cargo.toml").read_text(encoding="utf-8")
        )
        declared = workspace["workspace"]["package"]["version"]
        assert version == declared, (
            f"service reports {version!r} but the workspace declares {declared!r}"
        )


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
        client.get("/vms/list")
        text = client.get_text("/service-logs")
        assert isinstance(text, str) and text, "service-logs returned empty"
        assert len(text) > 10, f"service-logs implausibly short: {text!r}"
        # Both the daemon lifecycle (`capsem_service`) and its HTTP boundary
        # (`service`) are service-owned structured evidence. A bounded tail
        # need not retain the startup record after a busy shared test cohort.
        assert_service_log_evidence(text)


class TestReloadConfig:

    def test_profile_reload_no_instances(self, client):
        """/profiles/{profile_id}/reload succeeds with instances: 0 when no VMs are running."""
        # Make sure no VMs are running first.
        client.post("/purge", {"all": True})

        resp = client.post("/profiles/code/reload", {})
        assert resp is not None, "profile reload returned no body"
        assert resp.get("success") is True, f"profile reload failed: {resp}"
        assert resp.get("reloaded") == 0, (
            f"expected 0 reloaded, got {resp.get('reloaded')}: {resp}"
        )

    def test_retired_global_reload_config_route_is_removed(self, client):
        assert client.post("/reload-config", {}) is None
