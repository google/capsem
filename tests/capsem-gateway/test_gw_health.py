"""Gateway health endpoint tests.

GET / must be accessible without authentication (liveness probe).
"""

import re
import subprocess

import pytest

pytestmark = pytest.mark.gateway


class TestHealthEndpoint:

    def test_health_returns_200_without_auth(self, gateway_env):
        """GET / with no Authorization header returns 200."""
        result = subprocess.run(
            ["curl", "-s", "--max-time", "5",
             f"http://127.0.0.1:{gateway_env.port}/"],
            capture_output=True, text=True, timeout=10,
        )
        assert result.returncode == 0
        import json
        data = json.loads(result.stdout)
        assert data["ok"] is True
        assert "version" in data
        assert "service_socket" in data

    def test_health_version_is_semver(self, gateway_env):
        """Version field matches X.Y.Z pattern."""
        import json
        result = subprocess.run(
            ["curl", "-s", "--max-time", "5",
             f"http://127.0.0.1:{gateway_env.port}/"],
            capture_output=True, text=True, timeout=10,
        )
        data = json.loads(result.stdout)
        assert re.match(r"^\d+\.\d+\.\d+", data["version"]), (
            f"version {data['version']} is not semver"
        )

    def test_health_service_socket_path_present(self, gateway_env):
        """service_socket field is present and non-empty."""
        import json
        result = subprocess.run(
            ["curl", "-s", "--max-time", "5",
             f"http://127.0.0.1:{gateway_env.port}/"],
            capture_output=True, text=True, timeout=10,
        )
        data = json.loads(result.stdout)
        assert len(data["service_socket"]) > 0
