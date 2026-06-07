"""E2E test fixtures: real binaries, real startup, real CLI.

No ServiceInstance. No UdsHttpClient. Every operation goes through the
actual capsem CLI binary via subprocess, exactly as a user would.

The readiness check matches production: socket exists AND accepts
HTTP connections. If this diverges from what just run-service does,
we have a test infrastructure bug.
"""

import os
import json
import subprocess
import sys
import tempfile
import time
import uuid

import pytest

from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent.parent))

from helpers.constants import EXEC_READY_TIMEOUT
from helpers.profile_asset_fixture import find_asset, write_profile_home
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

    def __init__(self, *, capsem_home=None, assets_dir=None, extra_env=None):
        self.tmp_dir = Path(tempfile.mkdtemp(prefix="capsem-e2e-"))
        self.uds_path = self.tmp_dir / "service.sock"
        self.capsem_home = Path(capsem_home) if capsem_home else None
        self.assets_dir = Path(assets_dir) if assets_dir else None
        self.extra_env = dict(extra_env or {})
        self.env = None
        self.proc = None
        self._log_file = None
        self._stderr_file = None

    def start(self):
        sign_binary(PROCESS_BINARY)
        sign_binary(SERVICE_BINARY)

        arch = "arm64" if os.uname().machine == "arm64" else "x86_64"
        assets_dir = self.assets_dir or (ASSETS_DIR / arch)
        capsem_home = self.capsem_home or self.tmp_dir

        if self.capsem_home is None:
            asset_cache = self.tmp_dir / "assets"
            assets = {
                "vmlinuz": find_asset(assets_dir, "vmlinuz"),
                "initrd.img": find_asset(assets_dir, "initrd.img"),
                "rootfs.squashfs": find_asset(assets_dir, "rootfs.squashfs"),
            }
            write_profile_home(capsem_home, asset_cache, assets)
            assets_dir = asset_cache

        env = os.environ.copy()
        env["RUST_LOG"] = "capsem=debug"
        env["CAPSEM_RUN_DIR"] = str(self.tmp_dir)
        env["CAPSEM_HOME"] = str(capsem_home)
        env["CAPSEM_ASSETS_DIR"] = str(assets_dir)
        env.update(self.extra_env)
        self.env = env

        log_path = self.tmp_dir / "service.log"
        stderr_path = self.tmp_dir / "service.stderr.log"

        log_fd = os.open(log_path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o644)
        try:
            stderr_fd = os.open(stderr_path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o644)
            try:
                self.proc = subprocess.Popen(
                    [
                        str(SERVICE_BINARY),
                        "--uds-path", str(self.uds_path),
                        "--assets-dir", str(assets_dir),
                        "--process-binary", str(PROCESS_BINARY),
                        "--parent-pid", str(os.getpid()),
                        "--foreground",
                    ],
                    env=env,
                    stdout=log_fd,
                    stderr=stderr_fd,
                )
            finally:
                os.close(stderr_fd)
        finally:
            os.close(log_fd)

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
            cmd, capture_output=True, text=True, timeout=timeout, env=self.env,
        )

    def cli_ok(self, *args, timeout=60):
        """Run CLI and assert success. Returns CompletedProcess."""
        r = self.cli(*args, timeout=timeout)
        assert r.returncode == 0, (
            f"CLI failed: {' '.join(args)}\n"
            f"stdout: {r.stdout}\nstderr: {r.stderr}"
        )
        return r

    def api_json(self, method, path, payload=None, timeout=60):
        """Call the real service over its UDS HTTP API and decode JSON."""
        cmd = [
            "curl", "-sS", "--unix-socket", str(self.uds_path),
            "--max-time", str(timeout), "-X", method,
        ]
        body = None
        if payload is not None:
            cmd.extend(["-H", "Content-Type: application/json", "--data-binary", "@-"])
            body = json.dumps(payload)
        cmd.append("http://localhost" + path)
        r = subprocess.run(
            cmd, input=body, capture_output=True, text=True,
            timeout=timeout + 5, env=self.env,
        )
        assert r.returncode == 0, (
            f"HTTP {method} {path} failed\nstdout: {r.stdout}\nstderr: {r.stderr}"
        )
        return json.loads(r.stdout)

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


@pytest.fixture
def real_service_factory():
    """Factory for tests that need their own isolated real service."""
    return RealService
