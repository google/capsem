"""Gateway runtime file tests.

Verifies token, port, and PID files are written correctly and cleaned up.
"""

import os
import signal

import pytest

from helpers.gateway import GatewayInstance

pytestmark = pytest.mark.gateway


class TestRuntimeFiles:

    def test_token_file_exists(self, gateway_env):
        """gateway.token is written on startup."""
        token_path = gateway_env.run_dir / "gateway.token"
        assert token_path.exists()

    def test_token_file_permissions_600(self, gateway_env):
        """Token file has 0600 permissions (owner read/write only)."""
        token_path = gateway_env.run_dir / "gateway.token"
        mode = oct(token_path.stat().st_mode & 0o777)
        assert mode == "0o600", f"expected 0o600, got {mode}"

    def test_token_content_64_alphanumeric(self, gateway_env):
        """Token is 64 characters, all alphanumeric."""
        token = gateway_env.token
        assert len(token) == 64
        assert token.isalnum()

    def test_port_file_matches_actual_port(self, gateway_env):
        """Port file content matches the bound port."""
        port_path = gateway_env.run_dir / "gateway.port"
        assert port_path.exists()
        port = int(port_path.read_text().strip())
        assert port == gateway_env.port

    def test_pid_file_contains_running_pid(self, gateway_env):
        """PID file contains a PID that is actually running."""
        pid_path = gateway_env.run_dir / "gateway.pid"
        assert pid_path.exists()
        pid = int(pid_path.read_text().strip())
        # Verify process is alive
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            pytest.fail(f"PID {pid} from pid file is not running")

    def test_cleanup_on_shutdown(self, mock_service):
        """Runtime files are removed on clean shutdown."""
        gw = GatewayInstance(uds_path=mock_service.socket_path)
        gw.start()
        run_dir = gw.run_dir

        token_path = run_dir / "gateway.token"
        port_path = run_dir / "gateway.port"
        pid_path = run_dir / "gateway.pid"

        # Files exist while running
        assert token_path.exists()
        assert port_path.exists()
        assert pid_path.exists()

        # Stop triggers cleanup
        gw.stop()

        # Files should be removed
        assert not token_path.exists(), "token file not cleaned up"
        assert not port_path.exists(), "port file not cleaned up"
        assert not pid_path.exists(), "pid file not cleaned up"
