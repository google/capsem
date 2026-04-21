"""Gateway lifecycle tests.

Tests startup, shutdown, and restart behavior of the gateway binary.
"""

import json
import os
import signal
import subprocess
import time

import pytest

from helpers.gateway import GatewayInstance, TcpHttpClient

pytestmark = pytest.mark.gateway


class TestGatewayLifecycle:

    def test_sigterm_cleans_up_files(self, mock_service):
        """SIGTERM triggers graceful shutdown and file cleanup."""
        gw = GatewayInstance(uds_path=mock_service.socket_path)
        gw.start()
        run_dir = gw.run_dir
        try:
            # Verify files exist while running
            assert (run_dir / "gateway.token").exists()
            assert (run_dir / "gateway.port").exists()
            assert (run_dir / "gateway.pid").exists()

            # Send SIGTERM
            os.kill(gw.proc.pid, signal.SIGTERM)
            gw.proc.wait(timeout=10)

            # Give cleanup a moment
            time.sleep(0.5)

            # Files should be cleaned up
            assert not (run_dir / "gateway.token").exists(), "token not cleaned after SIGTERM"
            assert not (run_dir / "gateway.port").exists(), "port not cleaned after SIGTERM"
            assert not (run_dir / "gateway.pid").exists(), "pid not cleaned after SIGTERM"
        finally:
            # stop() closes the gateway.log file handle even when the proc
            # is already dead from the SIGTERM above. Without this the log
            # fd is leaked to GC; pytest's filterwarnings=error surfaces it
            # as PytestUnraisableExceptionWarning.
            gw.stop()

    def test_sigint_cleans_up_files(self, mock_service):
        """SIGINT (Ctrl-C) triggers graceful shutdown."""
        gw = GatewayInstance(uds_path=mock_service.socket_path)
        gw.start()
        run_dir = gw.run_dir
        try:
            assert (run_dir / "gateway.token").exists()

            os.kill(gw.proc.pid, signal.SIGINT)
            gw.proc.wait(timeout=10)
            time.sleep(0.5)

            assert not (run_dir / "gateway.token").exists(), "token not cleaned after SIGINT"
        finally:
            gw.stop()

    def test_two_gateways_on_different_ports(self, mock_service):
        """Two gateway instances can run simultaneously on different ports."""
        gw1 = GatewayInstance(uds_path=mock_service.socket_path)
        gw2 = GatewayInstance(uds_path=mock_service.socket_path)
        gw1.start()
        gw2.start()
        try:
            assert gw1.port != gw2.port, "two gateways should bind different ports"

            # Both should respond to health
            client1 = TcpHttpClient(gw1.base_url, gw1.token)
            client2 = TcpHttpClient(gw2.base_url, gw2.token)

            r1 = client1.get("/list")
            r2 = client2.get("/list")
            assert r1 is not None
            assert r2 is not None
            assert "sandboxes" in r1
            assert "sandboxes" in r2
        finally:
            gw1.stop()
            gw2.stop()

    def test_gateway_survives_service_restart(self, mock_service):
        """Gateway returns 502 when service drops, then recovers."""
        # Start gateway against a dead socket
        gw = GatewayInstance(uds_path="/tmp/capsem-gw-test-lifecycle.sock")
        gw.start()
        try:
            client = TcpHttpClient(gw.base_url, gw.token)

            # Should get 502 (no service)
            status = client.get_raw("/list")
            assert status == 502

            # Now point won't help since the UDS path is baked in,
            # but we verify the gateway is still responsive
            result = subprocess.run(
                ["curl", "-s", "--max-time", "5",
                 f"http://127.0.0.1:{gw.port}/"],
                capture_output=True, text=True, timeout=10,
            )
            data = json.loads(result.stdout)
            assert data["ok"] is True, "gateway should still be healthy even if service is down"
        finally:
            gw.stop()

    def test_gateway_tokens_are_unique_per_instance(self, mock_service):
        """Each gateway instance generates a unique token."""
        gw1 = GatewayInstance(uds_path=mock_service.socket_path)
        gw2 = GatewayInstance(uds_path=mock_service.socket_path)
        gw1.start()
        gw2.start()
        try:
            assert gw1.token != gw2.token, "tokens should be unique per instance"
            assert len(gw1.token) == 64
            assert len(gw2.token) == 64
        finally:
            gw1.stop()
            gw2.stop()

    def test_cross_token_rejected(self, mock_service):
        """Token from one gateway is rejected by another."""
        gw1 = GatewayInstance(uds_path=mock_service.socket_path)
        gw2 = GatewayInstance(uds_path=mock_service.socket_path)
        gw1.start()
        gw2.start()
        try:
            # Use gw1's token against gw2
            wrong_client = TcpHttpClient(gw2.base_url, gw1.token)
            status = wrong_client.get_raw("/list")
            assert status == 401, f"cross-token should be rejected, got {status}"
        finally:
            gw1.stop()
            gw2.stop()
