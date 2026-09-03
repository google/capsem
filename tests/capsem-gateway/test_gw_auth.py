"""Gateway authentication tests.

All endpoints except GET / require a valid Bearer token.
"""

import pytest
from helpers.gateway import TcpHttpClient

pytestmark = pytest.mark.gateway


class TestAuthAcceptance:

    def test_valid_token_proxies_request(self, gw_client):
        """GET /vms/list with valid Bearer token returns 200."""
        resp = gw_client.get("/vms/list")
        assert resp is not None
        assert "sandboxes" in resp

    def test_no_auth_header_returns_401(self, gateway_env):
        """GET /vms/list without Authorization header returns 401."""
        status = TcpHttpClient(gateway_env.base_url, gateway_env.token).call("GET", "/vms/list", use_auth=False)[0]
        assert status == 401

    def test_wrong_token_returns_401(self, gateway_env):
        """GET /vms/list with wrong Bearer token returns 401."""
        status = TcpHttpClient(gateway_env.base_url, gateway_env.token).call("GET", "/vms/list", headers={"Authorization": "Bearer wrong-token-value"}, use_auth=False)[0]
        assert status == 401

    def test_basic_auth_returns_401(self, gateway_env):
        """Basic auth is not accepted."""
        status = TcpHttpClient(gateway_env.base_url, gateway_env.token).call("GET", "/vms/list", headers={"Authorization": "Basic dG9rOg=="}, use_auth=False)[0]
        assert status == 401

    def test_bearer_no_space_returns_401(self, gateway_env):
        """'Bearertoken' (no space) is rejected."""
        status = TcpHttpClient(gateway_env.base_url, gateway_env.token).call("GET", "/vms/list", headers={"Authorization": f"Bearer{gateway_env.token}"}, use_auth=False)[0]
        assert status == 401

    def test_empty_bearer_returns_401(self, gateway_env):
        """'Bearer ' (empty token) is rejected."""
        status = TcpHttpClient(gateway_env.base_url, gateway_env.token).call("GET", "/vms/list", headers={"Authorization": "Bearer "}, use_auth=False)[0]
        assert status == 401

    def test_post_to_root_requires_auth(self, gateway_env):
        """POST / (not GET) requires auth."""
        status = TcpHttpClient(gateway_env.base_url, gateway_env.token).call("POST", "/", use_auth=False)[0]
        assert status == 401
