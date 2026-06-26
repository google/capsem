"""Gateway /status endpoint tests.

GET /status returns aggregated system health for tray polling.
"""


import pytest

pytestmark = pytest.mark.gateway


class TestStatusEndpoint:

    def test_status_returns_aggregated_response(self, gw_client):
        """GET /status returns full status JSON."""
        resp = gw_client.get("/status")
        assert resp is not None
        assert resp.get("service") == "running"
        assert "gateway_version" in resp
        assert "vm_count" in resp
        assert "vms" in resp
        assert "resource_summary" in resp

    def test_status_vm_count_matches_vms_array(self, gw_client):
        """vm_count field equals length of vms array."""
        resp = gw_client.get("/status")
        assert resp["vm_count"] == len(resp["vms"])

    def test_status_resource_summary_present(self, gw_client):
        """resource_summary has expected fields when service is running."""
        resp = gw_client.get("/status")
        rs = resp.get("resource_summary")
        assert rs is not None
        assert "total_ram_mb" in rs
        assert "total_cpus" in rs
        assert "running_count" in rs
        assert "stopped_count" in rs
        assert rs["total_ram_mb"] > 0
        assert rs["total_cpus"] > 0

    def test_status_includes_profile_catalog_and_manifest_provenance(self, gw_client):
        """GET /status preserves profile readiness and installed manifest provenance."""
        resp = gw_client.get("/status")
        profiles = resp.get("profiles")
        assert profiles is not None
        assert profiles["source"] == "directory"
        assert profiles["profile_count"] == 2
        assert profiles["ready_count"] == 1

        manifest = profiles["asset_manifest"]
        assert manifest["origin"] == "package"
        assert manifest["origin_source"] == "file:///tmp/corp/manifest.json"
        assert manifest["origin_path"].endswith("/manifest-origin.json")
        assert manifest["blake3"] == "0123456789abcdef"
        assert manifest["validation_status"] == "valid"
        assert manifest["refresh_policy"] == "24h"
        assert manifest["assets_current"] == "2026.0613.1"
        assert manifest["binaries_current"] == "1.3.0"

        by_id = {profile["id"]: profile for profile in profiles["profiles"]}
        assert by_id["code"]["ready"] is True
        assert by_id["code"]["asset_count"] == 3
        assert by_id["co-work"]["ready"] is False
        assert by_id["co-work"]["missing_assets"][0]["kind"] == "rootfs"

    def test_status_caches_within_ttl(self, gw_client):
        """Two rapid calls return identical data (cache TTL is 2s)."""
        resp1 = gw_client.get("/status")
        resp2 = gw_client.get("/status")
        # Same data (cache hit)
        assert resp1["vm_count"] == resp2["vm_count"]
        assert resp1["service"] == resp2["service"]
