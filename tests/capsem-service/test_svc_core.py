"""Core no-state service endpoints: /version, /stats, /service-logs, policy reload."""

import subprocess

import pytest
from helpers.service import SERVICE_BINARY
from log_streams import assert_service_log_evidence

pytestmark = pytest.mark.integration


class TestVersion:

    def test_version_returns_string(self, client):
        resp = client.get("/version")
        assert resp is not None
        version = resp.get("version")
        assert isinstance(version, str) and version, f"empty version: {resp}"
        # The real property is that the daemon reports the version it was
        # built from, so the authority is the binary the fixture launched, not
        # Cargo.toml: a profile release qualifies against the published
        # package while the source has already moved on, and comparing with
        # the workspace failed the 0.6.6 code-profile release on a correct
        # 0.6.5 service. Whether that binary is fresh is the artifact gate's
        # question, not this route's.
        built = subprocess.run(
            [str(SERVICE_BINARY), "--version"],
            capture_output=True,
            text=True,
            timeout=30,
            check=True,
        ).stdout.split()[-1]
        assert version == built, (
            f"service reports {version!r} but {SERVICE_BINARY} --version is {built!r}"
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

    def test_policy_reload_no_instances(self, client):
        """/corp/reload succeeds with reloaded: 0 when no VMs are running."""
        # Make sure no VMs are running first.
        client.post("/purge", {"all": True})

        resp = client.post("/corp/reload", {})
        assert resp is not None, "policy reload returned no body"
        assert resp.get("success") is True, f"policy reload failed: {resp}"
        assert resp.get("reloaded") == 0, (
            f"expected 0 reloaded, got {resp.get('reloaded')}: {resp}"
        )

    def test_retired_reload_routes_are_removed(self, client):
        assert client.post("/reload-config", {}) is None
