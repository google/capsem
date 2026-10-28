"""Gateway runtime file tests.

Verifies token, port, and PID files are written correctly and cleaned up.
"""

import os

import pytest
from helpers.gateway import GatewayInstance, _runtime_connection

pytestmark = pytest.mark.gateway


@pytest.mark.parametrize(
    ("token", "port"),
    [
        ("", ""),
        ("", "19222"),
        ("a" * 63, "19222"),
        ("a" * 65, "19222"),
        ("a" * 63 + "_", "19222"),
        ("a" * 64 + "\n", "19222"),
        (" " + "a" * 64, "19222"),
        ("é" * 64, "19222"),
        ("a" * 64, ""),
        ("a" * 64, "0"),
        ("a" * 64, "65536"),
        ("a" * 64, "-1"),
        ("a" * 64, "1x"),
        ("a" * 64, "١٢"),
    ],
)
def test_prepared_runtime_files_are_not_gateway_readiness(tmp_path, token, port):
    """Real files exist before confinement; only complete publication is ready."""
    (tmp_path / "gateway.token").write_text(token)
    (tmp_path / "gateway.port").write_text(port)
    assert _runtime_connection(tmp_path) is None


def test_runtime_readiness_requires_the_final_complete_token(tmp_path):
    assert _runtime_connection(tmp_path) is None
    (tmp_path / "gateway.port").write_text("19222")
    (tmp_path / "gateway.token").write_text("")
    assert _runtime_connection(tmp_path) is None
    (tmp_path / "gateway.token").write_text("a" * 63)
    assert _runtime_connection(tmp_path) is None
    (tmp_path / "gateway.token").write_text("a" * 64)
    assert _runtime_connection(tmp_path) == ("a" * 64, 19222)


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
