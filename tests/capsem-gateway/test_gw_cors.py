"""Gateway CORS tests.

Browser fetch needs CORS headers or requests fail.
"""

import json
import subprocess

import pytest

pytestmark = pytest.mark.gateway


class TestCORS:

    def test_cors_headers_on_health(self, gateway_env):
        """GET / with Origin header includes Access-Control-Allow-Origin."""
        result = subprocess.run(
            ["curl", "-s", "-D", "-", "--max-time", "5",
             "-H", "Origin: http://localhost:5173",
             f"http://127.0.0.1:{gateway_env.port}/"],
            capture_output=True, text=True, timeout=10,
        )
        headers = result.stdout.lower()
        assert "access-control-allow-origin" in headers

    def test_cors_preflight_options_no_auth(self, gateway_env):
        """OPTIONS preflight is handled by CORS layer without auth."""
        result = subprocess.run(
            ["curl", "-s", "-o", "/dev/null", "-w", "%{http_code}",
             "--max-time", "5",
             "-X", "OPTIONS",
             "-H", "Origin: http://localhost:5173",
             "-H", "Access-Control-Request-Method: GET",
             f"http://127.0.0.1:{gateway_env.port}/list"],
            capture_output=True, text=True, timeout=10,
        )
        status = result.stdout.strip()
        # CORS layer responds to preflight before auth -- should be 200, not 401
        assert status == "200", f"CORS preflight returned {status}, expected 200"

    def test_cors_on_authenticated_endpoint(self, gateway_env):
        """Authenticated request with Origin header gets CORS response headers."""
        result = subprocess.run(
            ["curl", "-s", "-D", "-", "--max-time", "5",
             "-H", f"Authorization: Bearer {gateway_env.token}",
             "-H", "Origin: http://localhost:5173",
             f"http://127.0.0.1:{gateway_env.port}/list"],
            capture_output=True, text=True, timeout=10,
        )
        headers = result.stdout.lower()
        assert "access-control-allow-origin" in headers
