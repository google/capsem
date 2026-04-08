"""Gateway authentication tests.

All endpoints except GET / require a valid Bearer token.
"""

import json
import subprocess

import pytest

pytestmark = pytest.mark.gateway


class TestAuthAcceptance:

    def test_valid_token_proxies_request(self, gw_client):
        """GET /list with valid Bearer token returns 200."""
        resp = gw_client.get("/list")
        assert resp is not None
        assert "sandboxes" in resp

    def test_no_auth_header_returns_401(self, gateway_env):
        """GET /list without Authorization header returns 401."""
        result = subprocess.run(
            ["curl", "-s", "-o", "/dev/null", "-w", "%{http_code}",
             "--max-time", "5",
             f"http://127.0.0.1:{gateway_env.port}/list"],
            capture_output=True, text=True, timeout=10,
        )
        assert result.stdout.strip() == "401"

    def test_wrong_token_returns_401(self, gateway_env):
        """GET /list with wrong Bearer token returns 401."""
        result = subprocess.run(
            ["curl", "-s", "-o", "/dev/null", "-w", "%{http_code}",
             "--max-time", "5",
             "-H", "Authorization: Bearer wrong-token-value",
             f"http://127.0.0.1:{gateway_env.port}/list"],
            capture_output=True, text=True, timeout=10,
        )
        assert result.stdout.strip() == "401"

    def test_basic_auth_returns_401(self, gateway_env):
        """Basic auth is not accepted."""
        result = subprocess.run(
            ["curl", "-s", "-o", "/dev/null", "-w", "%{http_code}",
             "--max-time", "5",
             "-H", "Authorization: Basic dG9rOg==",
             f"http://127.0.0.1:{gateway_env.port}/list"],
            capture_output=True, text=True, timeout=10,
        )
        assert result.stdout.strip() == "401"

    def test_bearer_no_space_returns_401(self, gateway_env):
        """'Bearertoken' (no space) is rejected."""
        result = subprocess.run(
            ["curl", "-s", "-o", "/dev/null", "-w", "%{http_code}",
             "--max-time", "5",
             "-H", f"Authorization: Bearer{gateway_env.token}",
             f"http://127.0.0.1:{gateway_env.port}/list"],
            capture_output=True, text=True, timeout=10,
        )
        assert result.stdout.strip() == "401"

    def test_empty_bearer_returns_401(self, gateway_env):
        """'Bearer ' (empty token) is rejected."""
        result = subprocess.run(
            ["curl", "-s", "-o", "/dev/null", "-w", "%{http_code}",
             "--max-time", "5",
             "-H", "Authorization: Bearer ",
             f"http://127.0.0.1:{gateway_env.port}/list"],
            capture_output=True, text=True, timeout=10,
        )
        assert result.stdout.strip() == "401"

    def test_post_to_root_requires_auth(self, gateway_env):
        """POST / (not GET) requires auth."""
        result = subprocess.run(
            ["curl", "-s", "-o", "/dev/null", "-w", "%{http_code}",
             "--max-time", "5", "-X", "POST",
             f"http://127.0.0.1:{gateway_env.port}/"],
            capture_output=True, text=True, timeout=10,
        )
        assert result.stdout.strip() == "401"
