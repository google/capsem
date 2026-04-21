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
GATEWAY_BINARY = PROJECT_ROOT / "target/debug/capsem-gateway"
TRAY_BINARY = PROJECT_ROOT / "target/debug/capsem-tray"
ASSETS_DIR = PROJECT_ROOT / "assets"


ARTIFACT_MAX_FILE_BYTES = 25 * 1024 * 1024  # 25 MB hard cap per file
ARTIFACT_SKIP_NAMES = frozenset({
    # Multi-GB VM disk images -- regenerable from the build, would burn
    # disk at ~2 GB per failure and we've been there.
    "rootfs.img",
    "rootfs.img.backing",
})
ARTIFACT_MAX_KEPT_DIRS = 20  # rotate: keep only the N most-recent failure dirs


def preserve_tmp_dir_on_failure(tmp_dir):
    """Copy tmp_dir to test-artifacts/ when this worker saw any failure.

    Called by integration-test fixture teardowns BEFORE they rmtree the
    tmp dir, so service.log, sessions/<vm>/process.log, sessions/<vm>/serial.log,
    and session.db survive for post-mortem. No-op on clean sessions.

    Skip rules (see constants above):
      - Sockets / FIFOs -- shutil.copy2 can't read them.
      - Files named in `ARTIFACT_SKIP_NAMES` (rootfs.img etc.) -- regenerable
        multi-GB artifacts that exploded disk on a 100%-full macOS volume.
      - Any regular file larger than `ARTIFACT_MAX_FILE_BYTES` -- safety net
        for whatever large artifact I haven't thought of yet.
    Also rotates `test-artifacts/` after each preserve, keeping only the
    most recent `ARTIFACT_MAX_KEPT_DIRS` failure dirs.
    """
    try:
        from tests.conftest import FAILED_NODEIDS, ARTIFACTS_ROOT
    except ImportError:
        return
    tmp_dir = Path(tmp_dir)
    if not FAILED_NODEIDS or not tmp_dir.exists():
        return
    import stat as statmod
    import time
    worker = os.environ.get("PYTEST_XDIST_WORKER", "master")
    tag = FAILED_NODEIDS[-1].replace("/", "_").replace(":", "_")[:80]
    ts = time.strftime("%Y%m%d-%H%M%S")
    dest = ARTIFACTS_ROOT / f"{ts}-{worker}-{tag}" / tmp_dir.name

    def _skip_unsupported(src, names):
        src_path = Path(src)
        skip = []
        for name in names:
            if name in ARTIFACT_SKIP_NAMES:
                skip.append(name)
                continue
            try:
                st = (src_path / name).lstat()
            except OSError:
                continue
            if statmod.S_ISSOCK(st.st_mode) or statmod.S_ISFIFO(st.st_mode):
                skip.append(name)
                continue
            # Size cap only for regular files (directories recurse and
            # get sized per-file on the next call).
            if statmod.S_ISREG(st.st_mode) and st.st_size > ARTIFACT_MAX_FILE_BYTES:
                skip.append(name)
        return skip

    try:
        dest.mkdir(parents=True, exist_ok=True)
        shutil.copytree(tmp_dir, dest, ignore=_skip_unsupported, dirs_exist_ok=True)
        print(f"ARTIFACT: preserved {tmp_dir} -> {dest}", file=sys.stderr)
        _rotate_artifacts(ARTIFACTS_ROOT, ARTIFACT_MAX_KEPT_DIRS)
    except Exception as e:
        print(f"ARTIFACT: preserve failed for {tmp_dir}: {e}", file=sys.stderr)


def _rotate_artifacts(root, keep):
    """Delete oldest `test-artifacts/<...>` dirs beyond `keep` most-recent."""
    if not root.exists():
        return
    try:
        dirs = sorted(
            (p for p in root.iterdir() if p.is_dir()),
            key=lambda p: p.name,  # names begin with YYYYMMDD-HHMMSS so string sort == chronological
        )
        for stale in dirs[:-keep] if keep > 0 else []:
            shutil.rmtree(stale, ignore_errors=True)
    except OSError as e:
        print(f"ARTIFACT: rotation skipped: {e}", file=sys.stderr)


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
        sign_binary(GATEWAY_BINARY)
        sign_binary(TRAY_BINARY)

        arch = "arm64" if os.uname().machine == "arm64" else "x86_64"
        assets_dir = ASSETS_DIR / arch

        env = os.environ.copy()
        env["RUST_LOG"] = "debug"
        env["CAPSEM_RUN_DIR"] = str(self.tmp_dir)
        env["CAPSEM_HOME"] = str(self.tmp_dir)
        env["HOME"] = str(self.tmp_dir)

        log_path = self.tmp_dir / "service.log"
        print(f"SERVICE LOG: {log_path}")
        self._log_file = open(log_path, "w")

        # Deliberately omit --tray-binary: the tray is a user-facing macOS
        # menu bar icon and spawning it on every test instance flashes the
        # menu bar dozens of times during a full suite run. Companion
        # lifecycle tests exercise the tray via their own spawn.
        self.proc = subprocess.Popen(
            [
                str(SERVICE_BINARY),
                "--uds-path", str(self.uds_path),
                "--assets-dir", str(assets_dir),
                "--process-binary", str(PROCESS_BINARY),
                "--gateway-binary", str(GATEWAY_BINARY),
                "--gateway-port", "0",
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
        """Stop the service and clean up temporary directory.

        Gives the service enough time for graceful shutdown to reap every
        per-VM capsem-process child (SIGTERM -> 500ms grace -> SIGKILL
        survivors). SIGKILL here would skip that cleanup and orphan VMs.
        """
        if self.proc:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()
            self.proc = None

        if self._log_file:
            self._log_file.close()
            self._log_file = None

        preserve_tmp_dir_on_failure(self.tmp_dir)

        if self.tmp_dir.exists():
            shutil.rmtree(self.tmp_dir, ignore_errors=True)


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
