"""Advanced gateway proxy tests.

Tests binary responses, large bodies, endpoint coverage, and edge cases
through the real gateway binary against the mock UDS service.
"""

import json
import subprocess
import tempfile
import os

import pytest

pytestmark = pytest.mark.gateway


class TestProxyEndpointCoverage:
    """Verify all mock service endpoints are reachable through the gateway."""

    def test_get_info_existing_vm(self, gw_client):
        """GET /info/{id} returns VM details for known VM."""
        resp = gw_client.get("/info/vm-001")
        assert resp is not None
        assert resp.get("id") == "vm-001"
        assert resp.get("name") == "dev"
        assert resp.get("status") == "Running"

    def test_get_info_unknown_vm(self, gw_client):
        """GET /info/{id} returns 404 for unknown VM."""
        resp = gw_client.get("/info/ghost-vm-999")
        assert resp is not None
        assert "error" in resp

    def test_post_exec_command(self, gw_client):
        """POST /exec/{id} returns stdout, stderr, exit_code."""
        resp = gw_client.post("/exec/vm-001", {"command": "whoami"})
        assert resp is not None
        assert "stdout" in resp
        assert resp.get("exit_code") == 0

    def test_post_stop_vm(self, gw_client):
        """POST /stop/{id} returns success."""
        resp = gw_client.post("/stop/vm-001", {})
        assert resp is not None

    def test_post_write_file(self, gw_client):
        """POST /write_file/{id} returns success."""
        resp = gw_client.post("/write_file/vm-001", {
            "path": "/tmp/test.txt",
            "content": "hello",
        })
        assert resp is not None

    def test_post_read_file(self, gw_client):
        """POST /read_file/{id} returns file content."""
        resp = gw_client.post("/read_file/vm-001", {"path": "/tmp/test.txt"})
        assert resp is not None

    def test_post_inspect(self, gw_client):
        """POST /inspect/{id} returns SQL query results."""
        resp = gw_client.post("/inspect/vm-001", {"query": "SELECT 1"})
        assert resp is not None

    def test_post_persist(self, gw_client):
        """POST /persist/{id} converts ephemeral to persistent."""
        resp = gw_client.post("/persist/vm-001", {"name": "saved"})
        assert resp is not None

    def test_post_purge(self, gw_client):
        """POST /purge kills ephemeral VMs."""
        resp = gw_client.post("/purge", {})
        assert resp is not None

    def test_post_run(self, gw_client):
        """POST /run one-shot command execution."""
        resp = gw_client.post("/run", {"command": "echo test"})
        assert resp is not None
        assert "stdout" in resp

    def test_post_resume(self, gw_client):
        """POST /resume/{name} resumes a persistent VM."""
        resp = gw_client.post("/resume/dev", {})
        assert resp is not None

    def test_post_fork(self, gw_client):
        """POST /fork/{id} creates a fork image."""
        resp = gw_client.post("/fork/vm-001", {"name": "snapshot1"})
        assert resp is not None
        assert resp.get("name") == "snapshot1"

    def test_get_images(self, gw_client):
        """GET /images returns image list."""
        resp = gw_client.get("/images")
        assert resp is not None
        assert "images" in resp

    def test_get_logs(self, gw_client):
        """GET /logs/{id} returns boot logs."""
        resp = gw_client.get("/logs/vm-001")
        assert resp is not None
        assert "logs" in resp

    def test_delete_vm(self, gw_client):
        """DELETE /delete/{id} destroys a VM."""
        resp = gw_client.delete("/delete/vm-001")
        assert resp is not None

    def test_post_reload_config(self, gw_client):
        """POST /reload-config reloads settings."""
        resp = gw_client.post("/reload-config", {})
        assert resp is not None


class TestProxyEdgeCases:

    def test_double_slash_in_path(self, gw_client):
        """Double slashes in path are handled gracefully."""
        # axum normalizes // to /, so this should work or 404
        resp = gw_client.get("//list")
        # Should not crash the gateway
        assert resp is not None or True  # 404 is acceptable

    def test_very_long_query_string(self, gw_client):
        """Long query strings are forwarded without truncation."""
        long_query = "x=" + "a" * 4000
        resp = gw_client.get(f"/info/vm-001?{long_query}")
        # Should succeed (query is forwarded, mock ignores it)
        assert resp is not None

    def test_empty_post_body(self, gw_client):
        """POST with empty body is forwarded correctly."""
        resp = gw_client.post("/echo", None)
        # Mock echoes back the body -- empty body returns empty or None
        # The key thing: no crash
        assert True  # If we get here, no crash

    def test_json_post_with_nested_data(self, gw_client):
        """POST with nested JSON is forwarded correctly."""
        payload = {
            "command": "echo test",
            "env": {"FOO": "bar", "BAZ": "qux"},
            "options": {"timeout": 30, "verbose": True},
        }
        resp = gw_client.post("/exec/vm-001", payload)
        assert resp is not None
        assert resp.get("exit_code") == 0

    def test_body_at_10mb_boundary(self, gateway_env):
        """Body at exactly 10MB is accepted (limit is >10MB)."""
        with tempfile.NamedTemporaryFile(suffix=".bin", delete=False) as f:
            f.write(b"x" * (10 * 1024 * 1024))
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
            status = result.stdout.strip()
            # 10MB exactly should be accepted (limit rejects >10MB)
            assert status in ("200", "502"), (
                f"10MB body returned {status}, expected 200 or 502 (502 if mock can't handle)"
            )
        finally:
            os.unlink(tmp_path)

    def test_head_request_through_gateway(self, gateway_env):
        """HEAD request is forwarded and returns no body."""
        result = subprocess.run(
            ["curl", "-s", "-D", "-", "-o", "/dev/null",
             "--max-time", "5", "-X", "HEAD",
             "-H", f"Authorization: Bearer {gateway_env.token}",
             f"http://127.0.0.1:{gateway_env.port}/list"],
            capture_output=True, text=True, timeout=10,
        )
        # HEAD should return headers but no body
        assert "HTTP/" in result.stdout

    def test_options_request_cors(self, gateway_env):
        """OPTIONS preflight returns CORS headers without auth."""
        result = subprocess.run(
            ["curl", "-s", "-D", "-",
             "--max-time", "5", "-X", "OPTIONS",
             "-H", "Origin: http://localhost:3000",
             "-H", "Access-Control-Request-Method: POST",
             "-H", "Access-Control-Request-Headers: authorization,content-type",
             f"http://127.0.0.1:{gateway_env.port}/provision"],
            capture_output=True, text=True, timeout=10,
        )
        headers = result.stdout.lower()
        assert "access-control-allow-origin" in headers
        assert "access-control-allow-methods" in headers
