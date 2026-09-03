"""Gateway CORS tests.

Browser fetch needs CORS headers or requests fail.
"""

import pytest
from helpers.gateway import TcpHttpClient

pytestmark = pytest.mark.gateway


class TestCORS:

    def test_cors_headers_on_health(self, gateway_env):
        """GET / with Origin header includes Access-Control-Allow-Origin."""
        _, headers, _ = TcpHttpClient(gateway_env.base_url, gateway_env.token).call(
            "GET", "/", headers={"Origin": "http://localhost:5173"}, use_auth=False
        )
        assert "access-control-allow-origin" in headers

    def test_cors_preflight_options_no_auth(self, gateway_env):
        """OPTIONS preflight is handled by CORS layer without auth."""
        status = TcpHttpClient(gateway_env.base_url, gateway_env.token).call(
            "OPTIONS",
            "/vms/list",
            headers={"Origin": "http://localhost:5173", "Access-Control-Request-Method": "GET"},
            use_auth=False,
        )[0]
        # CORS layer responds to preflight before auth -- should be 200, not 401
        assert status == 200, f"CORS preflight returned {status}, expected 200"

    def test_cors_on_authenticated_endpoint(self, gateway_env):
        """Authenticated request with Origin header gets CORS response headers."""
        _, headers, _ = TcpHttpClient(gateway_env.base_url, gateway_env.token).call("GET", "/vms/list", headers={"Origin": "http://localhost:5173"})
        assert "access-control-allow-origin" in headers
