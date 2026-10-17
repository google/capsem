"""Advanced gateway /status endpoint tests.

Tests refresh behavior, response schema, and edge cases through the real
gateway binary.
"""

import pytest

pytestmark = pytest.mark.gateway


class TestStatusRefreshBehavior:

    def test_status_reads_fresh_service_state(self, gw_client, mock_vms):
        """Every /status request begins a fresh service read."""
        resp1 = gw_client.get("/status")
        assert resp1 is not None

        vm_id = "33333333-3333-4333-8333-333333333333"
        mock_vms[vm_id] = {
            "id": vm_id,
            "pid": 300,
            "name": "fresh",
            "status": "Running",
            "persistent": False,
            "ram_mb": 1024,
            "cpus": 1,
            "version": "0.16.1",
        }
        try:
            resp2 = gw_client.get("/status")
        finally:
            mock_vms.pop(vm_id)

        assert resp2 is not None
        assert resp2["service"] == "running"
        assert resp2["vm_count"] == resp1["vm_count"] + 1

    def test_status_version_is_semver(self, gw_client):
        """gateway_version field is a valid semver string."""
        import re
        resp = gw_client.get("/status")
        assert resp is not None
        version = resp.get("gateway_version", "")
        assert re.match(r"^\d+\.\d+\.\d+", version), (
            f"gateway_version '{version}' is not semver"
        )

    def test_status_vm_summaries_have_required_fields(self, gw_client):
        """Each VM summary in the response has id, status, persistent."""
        resp = gw_client.get("/status")
        assert resp is not None
        for vm in resp.get("vms", []):
            assert "id" in vm, f"VM summary missing 'id': {vm}"
            assert "status" in vm, f"VM summary missing 'status': {vm}"
            assert "persistent" in vm, f"VM summary missing 'persistent': {vm}"

    def test_status_resource_summary_aggregation(self, gw_client):
        """resource_summary totals match the individual VM data."""
        resp = gw_client.get("/status")
        assert resp is not None
        rs = resp.get("resource_summary")
        assert rs is not None
        # Mock has 2 VMs, both Running
        assert rs["running_count"] + rs["stopped_count"] == resp["vm_count"]
        assert rs["total_ram_mb"] > 0
        assert rs["total_cpus"] > 0


class TestStatusServiceDown:

    def test_status_returns_unavailable_when_service_down(self):
        """GET /status returns service=unavailable when UDS is dead."""
        from helpers.gateway import GatewayInstance, TcpHttpClient

        gw = GatewayInstance(uds_path="/tmp/capsem-gw-test-dead-service.sock")
        gw.start()
        try:
            client = TcpHttpClient(gw.base_url, gw.token)
            resp = client.get("/status")
            assert resp is not None
            assert resp["service"] == "unavailable"
            assert resp["vm_count"] == 0
            assert resp["resource_summary"] is None
        finally:
            gw.stop()
