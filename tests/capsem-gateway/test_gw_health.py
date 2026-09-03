"""Gateway health endpoint tests.

GET / must be accessible without authentication (liveness probe).
"""

import re

import pytest
from helpers.gateway import TcpHttpClient

pytestmark = pytest.mark.gateway


class TestHealthEndpoint:

    def test_health_returns_200_without_auth(self, gateway_env):
        """GET / with no Authorization header returns 200."""
        _, data = TcpHttpClient(gateway_env.base_url, gateway_env.token).call_json("GET", "/", use_auth=False)
        assert data["ok"] is True
        assert "version" in data
        assert "service_socket" in data

    def test_health_version_is_semver(self, gateway_env):
        """Version field matches X.Y.Z pattern."""
        _, data = TcpHttpClient(gateway_env.base_url, gateway_env.token).call_json("GET", "/", use_auth=False)
        assert re.match(r"^\d+\.\d+\.\d+", data["version"]), (
            f"version {data['version']} is not semver"
        )

    def test_health_service_socket_path_present(self, gateway_env):
        """service_socket field is present and non-empty."""
        _, data = TcpHttpClient(gateway_env.base_url, gateway_env.token).call_json("GET", "/", use_auth=False)
        assert len(data["service_socket"]) > 0
