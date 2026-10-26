"""Gateway CORS tests.

Browser fetch needs CORS headers or requests fail.
"""

import pytest
from helpers.gateway import TcpHttpClient

pytestmark = pytest.mark.gateway


@pytest.mark.parametrize("origin,allowed", [("tauri://localhost", True), ("http://localhost:5173", False)])
class TestCORS:

    def test_cors_headers_on_health(self, gateway_env, origin, allowed):
        """GET / with Origin header includes Access-Control-Allow-Origin."""
        _, headers, _ = TcpHttpClient(gateway_env.base_url, gateway_env.token).call(
            "GET", "/", headers={"Origin": origin}, use_auth=False
        )
        if allowed:
            assert headers.get("access-control-allow-origin") == origin
        else:
            assert "access-control-allow-origin" not in headers

    def test_cors_preflight_options_no_auth(self, gateway_env, origin, allowed):
        """OPTIONS preflight is handled by CORS layer without auth."""
        status, headers, _ = TcpHttpClient(gateway_env.base_url, gateway_env.token).call(
            "OPTIONS",
            "/vms/list",
            headers={"Origin": origin, "Access-Control-Request-Method": "GET"},
            use_auth=False,
        )
        # CORS layer responds to preflight before auth -- should be 200, not 401
        assert status == 200, f"CORS preflight returned {status}, expected 200"
        if allowed:
            assert headers.get("access-control-allow-origin") == origin
        else:
            assert "access-control-allow-origin" not in headers

    def test_cors_on_authenticated_endpoint(self, gateway_env, origin, allowed):
        """Authenticated request with Origin header gets CORS response headers."""
        _, headers, _ = TcpHttpClient(gateway_env.base_url, gateway_env.token).call("GET", "/vms/list", headers={"Origin": origin})
        if allowed:
            assert headers.get("access-control-allow-origin") == origin
        else:
            assert "access-control-allow-origin" not in headers
