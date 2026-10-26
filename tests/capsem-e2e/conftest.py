"""E2E test fixtures: real binaries, real startup, real CLI.

No ServiceInstance. No UdsHttpClient. Every operation goes through the
actual capsem CLI binary via subprocess, exactly as a user would.

Readiness uses the authenticated CLI path, including gateway startup.
"""

import os
import shutil
import subprocess
import sys
import time
import uuid
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).parent.parent))

from helpers import failures
from helpers.constants import ASSETS_DIR, BIN_DIR, EXEC_READY_TIMEOUT
from helpers.mock_server import mock_server_lock_directory
from helpers.service import make_service_home_run_dirs, preserve_tmp_dir_on_failure
from helpers.sign import sign_binary
from log_streams import read_log_stream

SERVICE_BINARY = BIN_DIR / "capsem-service"
PROCESS_BINARY = BIN_DIR / "capsem-process"
CLI_BINARY = BIN_DIR / "capsem"

pytestmark = pytest.mark.e2e


def _vm_name(prefix="e2e"):
    return f"{prefix}-{uuid.uuid4().hex[:8]}"


class RealService:
    """Start a real service and prove its authenticated CLI gateway is ready."""

    def __init__(self):
        # The installed layout: CAPSEM_HOME owns run/, and sessions/host.db
        # sits beside it. One directory for both put the host ledger in the run-wide
        # temporary parent, shared by every worker's service.
        self.home_dir, self.tmp_dir = make_service_home_run_dirs()
        self.uds_path = self.tmp_dir / f"service-{uuid.uuid4().hex[:8]}.sock"
        self.proc = None
        self._log_file = None
        self._stderr_file = None

    def start(self):
        failures.LIVE_HOMES.add(self.home_dir)
        sign_binary(PROCESS_BINARY)
        sign_binary(SERVICE_BINARY)

        arch = "arm64" if os.uname().machine == "arm64" else "x86_64"
        assets_dir = ASSETS_DIR / arch

        env = os.environ.copy()
        env["RUST_LOG"] = "capsem=debug"
        env["CAPSEM_RUN_DIR"] = str(self.tmp_dir)
        env["CAPSEM_HOME"] = str(self.home_dir)
        env["CAPSEM_CREDENTIAL_STORE_PATH"] = str(
            self.home_dir / "credential-store.json"
        )
        env["HOME"] = str(self.home_dir)

        log_path = self.tmp_dir / "service.log"
        stderr_path = self.tmp_dir / "service.stderr.log"
        self._log_file = open(log_path, "w")  # noqa: SIM115 -- handed to Popen; must outlive this statement
        self._stderr_file = open(stderr_path, "w")  # noqa: SIM115 -- handed to Popen; must outlive this statement

        self.proc = subprocess.Popen(
            [
                str(SERVICE_BINARY),
                "--uds-path", str(self.uds_path),
                "--assets-dir", str(assets_dir),
                "--process-binary", str(PROCESS_BINARY),
                "--gateway-port", "0",
                "--parent-pid", str(os.getpid()),
                "--foreground",
            ],
            env=env,
            stdout=self._log_file,
            stderr=self._stderr_file,
        )

        # The service socket can answer before its gateway is ready.
        start = time.time()
        while time.time() - start < 15:
            if self.uds_path.exists():
                try:
                    if self.cli("list", timeout=2).returncode == 0:
                        return
                except subprocess.TimeoutExpired:
                    pass
            time.sleep(0.5)

        self._dump_logs()
        self.stop()
        raise RuntimeError(
            f"capsem-service did not accept connections within 15s. "
            f"Logs: {self.tmp_dir}"
        )

    def stop(self):
        if self.proc:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()
        if self._log_file:
            self._log_file.close()
        if self._stderr_file:
            self._stderr_file.close()
        # The service is session-scoped; its home belongs to every test this
        # worker ran, not only to the last PYTEST_CURRENT_TEST value.
        preserve_tmp_dir_on_failure(self.home_dir, any_worker_failure=True)
        failures.LIVE_HOMES.discard(self.home_dir)
        shutil.rmtree(self.home_dir, ignore_errors=True)

    def cli(self, *args, timeout=60):
        """Run the real capsem CLI binary. Returns CompletedProcess."""
        cmd = [str(CLI_BINARY), "--uds-path", str(self.uds_path), *list(args)]
        return subprocess.run(
            cmd, capture_output=True, text=True, timeout=timeout, check=False,
            env={**os.environ, "CAPSEM_MOCK_SERVER_LOCK_DIR": str(mock_server_lock_directory())},
        )

    def cli_ok(self, *args, timeout=60):
        """Run CLI and assert success. Returns CompletedProcess."""
        r = self.cli(*args, timeout=timeout)
        assert r.returncode == 0, (
            f"CLI failed: {' '.join(args)}\n"
            f"stdout: {r.stdout}\nstderr: {r.stderr}"
        )
        return r

    def wait_exec_ready(self, vm_name, timeout=EXEC_READY_TIMEOUT):
        """Wait until a VM responds to exec via the real CLI.

        The server polls internally for VM readiness, so a single call with
        adequate timeout is sufficient.
        """
        r = self.cli("exec", "--timeout", str(timeout), vm_name, "echo ready",
                      timeout=timeout + 5)
        return r.returncode == 0 and "ready" in r.stdout

    def _dump_logs(self):
        for name in ["service.log", "service.stderr.log"]:
            # service.log is a rotated stream; reading the name alone dumps
            # nothing and makes a failure look like a silent one.
            text = read_log_stream(self.tmp_dir / name)
            if text:
                print(f"\n--- {name} ---\n{text}\n---", file=sys.stderr)


@pytest.fixture(scope="session")
def service():
    """A real capsem-service for the entire E2E session."""
    svc = RealService()
    svc.start()
    yield svc
    svc.stop()
