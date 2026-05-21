"""Gateway proxy forwarding tests.

Verifies that requests are correctly proxied from TCP to UDS.
"""

import json
import subprocess

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.gateway import GatewayInstance, TcpHttpClient

pytestmark = pytest.mark.gateway


class TestProxyForwarding:

    def test_get_list_through_gateway(self, gw_client):
        """GET /list returns mock VM list."""
        resp = gw_client.get("/list")
        assert resp is not None
        assert "sandboxes" in resp
        assert len(resp["sandboxes"]) == 2

    def test_post_provision_with_body(self, gw_client):
        """POST /provision with JSON body returns an id."""
        resp = gw_client.post("/provision", {"ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
        assert resp is not None
        assert "id" in resp

    def test_post_exec_returns_stdout(self, gw_client):
        """POST /exec/{id} returns command output."""
        resp = gw_client.post("/exec/vm-001", {"command": "echo hello"})
        assert resp is not None
        assert resp.get("exit_code") == 0
        assert "echo hello" in resp.get("stdout", "")

    def test_delete_through_gateway(self, gw_client):
        """DELETE /delete/{id} returns success."""
        resp = gw_client.delete("/delete/vm-001")
        assert resp is not None

    def test_preserves_query_string(self, gw_client):
        """Query parameters are preserved through proxy."""
        # Use /info with query -- mock doesn't use query but it must not crash
        resp = gw_client.get("/info/vm-001?detail=true")
        assert resp is not None
        assert resp.get("id") == "vm-001"

    def test_preserves_upstream_404(self, gw_client):
        """404 from upstream service is proxied as-is."""
        resp = gw_client.get("/info/ghost-vm-nonexistent")
        assert resp is not None
        assert "error" in str(resp).lower() or "not found" in str(resp).lower()


class TestProxySecurity:

    def test_502_when_service_down(self):
        """Gateway returns 502 when UDS service is unavailable."""
        gw = GatewayInstance(uds_path="/tmp/capsem-gw-test-no-such-service.sock")
        gw.start()
        try:
            client = TcpHttpClient(gw.base_url, gw.token)
            status = client.get_raw("/list")
            assert status == 502
        finally:
            gw.stop()

    def test_path_traversal_safe(self, gw_client):
        """Path traversal attempt doesn't crash or escape."""
        # axum normalizes /../ in paths, so this should resolve to /etc/passwd
        # or be rejected -- either way it must not leak host filesystem contents
        resp = gw_client.get("/info/../../../etc/passwd")
        # The mock will return a 404 (no such VM). The important thing is
        # it did NOT return actual /etc/passwd contents from the host.
        if resp is not None:
            assert "root:" not in str(resp), "host /etc/passwd leaked through proxy"

    def test_oversized_body_rejected(self, gateway_env):
        """Bodies larger than 10MB are rejected with 413."""
        import tempfile
        # Write 11MB to a temp file and POST it via --data-binary @file
        with tempfile.NamedTemporaryFile(suffix=".bin", delete=False) as f:
            f.write(b"x" * (11 * 1024 * 1024))
            tmp_path = f.name
        try:
            result = subprocess.run(
                ["curl", "-s", "-o", "/dev/null", "-w", "%{http_code}",
                 "--max-time", "30", "-X", "POST",
                 "-H", f"Authorization: Bearer {gateway_env.token}",
                 "-H", "Content-Type: application/octet-stream",
                 "--data-binary", f"@{tmp_path}",
                 f"http://127.0.0.1:{gateway_env.port}/echo"],
                capture_output=True, text=True, timeout=60,
            )
            assert result.stdout.strip() == "413"
        finally:
            import os
            os.unlink(tmp_path)
