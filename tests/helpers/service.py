"""Shared service startup helper for integration tests."""

import os
import shutil
import subprocess
import sys
import tempfile
import time
import uuid

from pathlib import Path

from .constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from .sign import sign_binary
from .uds_client import UdsHttpClient

PROJECT_ROOT = Path(__file__).parent.parent.parent
SERVICE_BINARY = PROJECT_ROOT / "target/debug/capsem-service"
PROCESS_BINARY = PROJECT_ROOT / "target/debug/capsem-process"
ASSETS_DIR = PROJECT_ROOT / "assets"


class ServiceInstance:
    """A running capsem-service instance on an isolated socket."""

    def __init__(self):
        self.tmp_dir = Path(tempfile.mkdtemp(prefix="capsem-test-"))
        self.uds_path = self.tmp_dir / f"service-{uuid.uuid4().hex[:8]}.sock"
        self.proc = None
        self._log_file = None

    def start(self):
        # Sign binaries before spawning (macOS needs virtualization entitlement)
        sign_binary(PROCESS_BINARY)
        sign_binary(SERVICE_BINARY)

        arch = "arm64" if os.uname().machine == "arm64" else "x86_64"
        assets_dir = ASSETS_DIR / arch

        env = os.environ.copy()
        env["RUST_LOG"] = "debug"
        env["CAPSEM_RUN_DIR"] = str(self.tmp_dir)

        log_path = self.tmp_dir / "service.log"
        print(f"SERVICE LOG: {log_path}")
        self._log_file = open(log_path, "w")

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
            stderr=self._log_file,
        )

        start = time.time()
        while time.time() - start < 15:
            if self.uds_path.exists():
                # Socket file exists -- verify server is actually accepting connections
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

        self.stop()
        if log_path.exists():
            print(f"\n--- SERVICE LOG ---\n{log_path.read_text()}\n---", file=sys.stderr)
        raise RuntimeError("capsem-service failed to accept connections within 15s")

    def client(self):
        return UdsHttpClient(self.uds_path)

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
        # shutil.rmtree(self.tmp_dir, ignore_errors=True)


def wait_exec_ready(client, vm_name, timeout=EXEC_READY_TIMEOUT):
    """Wait until a VM responds to exec.

    The server's handle_exec already polls internally for VM readiness,
    so a single call with adequate timeout is sufficient -- no client-side
    retry loop needed.
    """
    try:
        resp = client.post(
            f"/exec/{vm_name}",
            {"command": "echo ready", "timeout_secs": timeout},
            timeout=timeout + 5,
        )
        return resp is not None and "ready" in resp.get("stdout", "")
    except Exception:
        return False


def vm_name(prefix="test"):
    """Generate a unique VM name with the given prefix."""
    return f"{prefix}-{uuid.uuid4().hex[:8]}"
