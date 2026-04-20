"""E2E test fixtures: real binaries, real startup, real CLI.

No ServiceInstance. No UdsHttpClient. Every operation goes through the
actual capsem CLI binary via subprocess, exactly as a user would.

The readiness check matches production: socket exists AND accepts
HTTP connections. If this diverges from what just run-service does,
we have a test infrastructure bug.
"""

import os
import subprocess
import sys
import tempfile
import time
import uuid

import pytest

from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent.parent))

from helpers.constants import EXEC_READY_TIMEOUT
from helpers.service import preserve_tmp_dir_on_failure
from helpers.sign import sign_binary

PROJECT_ROOT = Path(__file__).parent.parent.parent
SERVICE_BINARY = PROJECT_ROOT / "target/debug/capsem-service"
PROCESS_BINARY = PROJECT_ROOT / "target/debug/capsem-process"
CLI_BINARY = PROJECT_ROOT / "target/debug/capsem"
MCP_BINARY = PROJECT_ROOT / "target/debug/capsem-mcp"
ASSETS_DIR = PROJECT_ROOT / "assets"

pytestmark = pytest.mark.e2e


def _vm_name(prefix="e2e"):
    return f"{prefix}-{uuid.uuid4().hex[:8]}"


class RealService:
    """Starts capsem-service the way just run-service does.

    Readiness check: socket file exists AND curl to /list succeeds.
    This is intentionally the same logic as the justfile run-service
    recipe. If they diverge, tests pass but the product breaks.
    """

    def __init__(self):
        self.tmp_dir = Path(tempfile.mkdtemp(prefix="capsem-e2e-"))
        self.uds_path = self.tmp_dir / f"service-{uuid.uuid4().hex[:8]}.sock"
        self.proc = None
        self._log_file = None
        self._stderr_file = None

    def start(self):
        sign_binary(PROCESS_BINARY)
        sign_binary(SERVICE_BINARY)

        arch = "arm64" if os.uname().machine == "arm64" else "x86_64"
        assets_dir = ASSETS_DIR / arch

        env = os.environ.copy()
        env["RUST_LOG"] = "capsem=debug"
        env["CAPSEM_RUN_DIR"] = str(self.tmp_dir)

        log_path = self.tmp_dir / "service.log"
        stderr_path = self.tmp_dir / "service.stderr.log"
        self._log_file = open(log_path, "w")
        self._stderr_file = open(stderr_path, "w")

        self.proc = subprocess.Popen(
            [
                str(SERVICE_BINARY),
                "--uds-path", str(self.uds_path),
                "--assets-dir", str(assets_dir),
                "--process-binary", str(PROCESS_BINARY),
                "--foreground",
            ],
            env=env,
            stdout=self._log_file,
            stderr=self._stderr_file,
        )

        # Readiness check: matches production (just run-service).
        # Socket file exists AND service responds to HTTP.
        start = time.time()
        while time.time() - start < 15:
            if self.uds_path.exists():
                try:
                    result = subprocess.run(
                        ["curl", "-s", "--unix-socket", str(self.uds_path),
                         "--max-time", "2", "http://localhost/list"],
                        capture_output=True, text=True, timeout=5,
                    )
                    if result.returncode == 0:
                        return
                except Exception:
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
        preserve_tmp_dir_on_failure(self.tmp_dir)

    def cli(self, *args, timeout=60):
        """Run the real capsem CLI binary. Returns CompletedProcess."""
        cmd = [str(CLI_BINARY), "--uds-path", str(self.uds_path)] + list(args)
        return subprocess.run(
            cmd, capture_output=True, text=True, timeout=timeout,
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
            p = self.tmp_dir / name
            if p.exists():
                print(f"\n--- {name} ---\n{p.read_text()}\n---",
                      file=sys.stderr)


@pytest.fixture(scope="session")
def service():
    """A real capsem-service for the entire E2E session."""
    svc = RealService()
    svc.start()
    yield svc
    svc.stop()
