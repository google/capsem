"""E2E service startup tests.

Tests the actual service binary startup path -- the same code that
runs when a user does just run-service. If these fail, just shell fails.
"""

import os
import socket
import subprocess
import signal
import time

import pytest

from .conftest import RealService
from helpers.constants import EXEC_READY_TIMEOUT

pytestmark = pytest.mark.e2e


class TestServiceStartup:

    def test_service_starts_and_accepts_connections(self):
        """Start service, verify it accepts connections (not just socket exists)."""
        svc = RealService()
        svc.start()  # raises RuntimeError if readiness check fails
        try:
            r = svc.cli("list")
            assert r.returncode == 0, f"list failed after startup: {r.stderr}"
        finally:
            svc.stop()

    def test_service_socket_accepts_raw_tcp(self):
        """The UDS socket must accept raw TCP connections."""
        svc = RealService()
        svc.start()
        try:
            sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            try:
                sock.connect(str(svc.uds_path))
            except ConnectionRefusedError:
                pytest.fail(
                    f"Socket exists at {svc.uds_path} but refuses connections"
                )
            finally:
                sock.close()
        finally:
            svc.stop()

    def test_service_replaces_stale_socket(self):
        """If a stale socket file exists, service replaces it and starts."""
        svc = RealService()
        # Create a fake stale socket file
        svc.uds_path.parent.mkdir(parents=True, exist_ok=True)
        svc.uds_path.touch()

        svc.start()  # must succeed despite stale file
        try:
            r = svc.cli("list")
            assert r.returncode == 0
        finally:
            svc.stop()

    def test_service_clean_shutdown(self):
        """Service terminates cleanly without orphaned processes."""
        svc = RealService()
        svc.start()
        pid = svc.proc.pid

        # Start a VM so there's a capsem-process child
        name = f"shutdown-{os.getpid()}"
        svc.cli("start", "--rm", "--name", name)
        svc.wait_exec_ready(name, timeout=EXEC_READY_TIMEOUT)

        svc.stop()
        assert svc.proc.returncode is not None, (
            f"Service process {pid} did not terminate"
        )

        # Give children a moment to exit
        time.sleep(1)

        # Check no orphaned capsem-process for our temp dir
        result = subprocess.run(
            ["pgrep", "-f", str(svc.tmp_dir)],
            capture_output=True, text=True,
        )
        assert result.returncode != 0, (
            f"Orphaned processes found after shutdown: {result.stdout}"
        )
