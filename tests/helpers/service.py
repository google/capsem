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
from .profile_asset_fixture import find_asset, write_profile_home
from .sign import sign_binary
from .uds_client import UdsHttpClient

PROJECT_ROOT = Path(__file__).parent.parent.parent
SERVICE_BINARY = PROJECT_ROOT / "target/debug/capsem-service"
PROCESS_BINARY = PROJECT_ROOT / "target/debug/capsem-process"
GATEWAY_BINARY = PROJECT_ROOT / "target/debug/capsem-gateway"
TRAY_BINARY = PROJECT_ROOT / "target/debug/capsem-tray"
ASSETS_DIR = Path(os.environ.get("CAPSEM_ASSETS_DIR", PROJECT_ROOT / "assets"))


ARTIFACT_MAX_FILE_BYTES = 25 * 1024 * 1024  # 25 MB hard cap per file
ARTIFACT_SKIP_NAMES = frozenset({
    # Multi-GB VM disk images -- regenerable from the build, would burn
    # disk at ~2 GB per failure and we've been there.
    "rootfs.img",
    "rootfs.img.backing",
    # VM memory checkpoints -- ~100MB+ per suspend, skip to save space.
    # The logs in the same directory are what we need for debugging.
    "checkpoint.vzsave",
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

    Uses a manual os.walk + per-file copy loop rather than shutil.copytree
    with an ignore filter. Past incidents showed copytree creating the
    destination subdirs (sessions/, persistent/) but leaving them empty
    when capsem-process was still alive and rewriting/deleting files
    concurrently during teardown. A per-file try/except isolates those
    transient errors so one flaky file doesn't vanish the entire subtree.

    Also rotates `test-artifacts/` after each preserve, keeping only the
    most recent `ARTIFACT_MAX_KEPT_DIRS` failure dirs.
    """
    try:
        from tests.conftest import FAILED_NODEIDS, ARTIFACTS_ROOT
    except ImportError:
        return
    tmp_dir = Path(tmp_dir)
    if not tmp_dir.exists():
        return
    # CAPSEM_TEST_PRESERVE_ALWAYS=1 forces preservation of every worker's
    # tmp_dir regardless of that worker's own failure state. Used during
    # concurrency investigations where a failure on worker B needs to be
    # correlated against what worker A was doing at the same time.
    force = os.environ.get("CAPSEM_TEST_PRESERVE_ALWAYS")
    if not force and not FAILED_NODEIDS:
        return
    import stat as statmod
    import time
    worker = os.environ.get("PYTEST_XDIST_WORKER", "master")
    if FAILED_NODEIDS:
        tag = FAILED_NODEIDS[-1].replace("/", "_").replace(":", "_")[:80]
    else:
        tag = "no-failures-on-this-worker"
    ts = time.strftime("%Y%m%d-%H%M%S")
    dest = ARTIFACTS_ROOT / f"{ts}-{worker}-{tag}" / tmp_dir.name

    copied = 0
    skipped_name = 0
    skipped_size = 0
    skipped_type = 0
    errors = []

    try:
        dest.mkdir(parents=True, exist_ok=True)
        # topdown=True so we can prune by emptying dirnames in-place if
        # needed; onerror catches listdir failures so a single unreadable
        # subdir doesn't abort the whole walk.
        def _on_walk_error(err):
            errors.append(f"walk {err.filename}: {err}")
        for src_dir, dirnames, filenames in os.walk(tmp_dir, topdown=True, onerror=_on_walk_error):
            src_path = Path(src_dir)
            rel = src_path.relative_to(tmp_dir)
            dst_dir = dest / rel
            try:
                dst_dir.mkdir(parents=True, exist_ok=True)
            except OSError as e:
                errors.append(f"mkdir {dst_dir}: {e}")
                continue
            for name in filenames:
                if name in ARTIFACT_SKIP_NAMES:
                    skipped_name += 1
                    continue
                src_file = src_path / name
                dst_file = dst_dir / name
                try:
                    st = src_file.lstat()
                except OSError as e:
                    errors.append(f"lstat {src_file}: {e}")
                    continue
                mode = st.st_mode
                if statmod.S_ISSOCK(mode) or statmod.S_ISFIFO(mode):
                    skipped_type += 1
                    continue
                if statmod.S_ISLNK(mode):
                    # Preserve as symlink -- don't chase the target.
                    # Dangling symlinks under concurrent teardown would
                    # otherwise fail copy2 and (with copytree) poison
                    # the whole subdir.
                    try:
                        target = os.readlink(src_file)
                        os.symlink(target, dst_file)
                        copied += 1
                    except OSError as e:
                        errors.append(f"symlink {src_file}: {e}")
                    continue
                if statmod.S_ISREG(mode) and st.st_size > ARTIFACT_MAX_FILE_BYTES:
                    skipped_size += 1
                    continue
                try:
                    shutil.copy2(src_file, dst_file)
                    copied += 1
                except OSError as e:
                    errors.append(f"copy {src_file}: {e}")
        print(
            f"ARTIFACT: preserved {tmp_dir} -> {dest} "
            f"(copied={copied} skipped_name={skipped_name} "
            f"skipped_size={skipped_size} skipped_type={skipped_type} "
            f"errors={len(errors)})",
            file=sys.stderr,
        )
        for err in errors[:10]:
            print(f"  ! {err}", file=sys.stderr)
        _rotate_artifacts(ARTIFACTS_ROOT, ARTIFACT_MAX_KEPT_DIRS)
    except Exception as e:
        print(f"ARTIFACT: preserve fatal for {tmp_dir}: {e}", file=sys.stderr)


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

    def __init__(self, extra_env=None, service_toml=None, pass_assets_dir=True, assets_dir=None):
        self.tmp_dir = Path(tempfile.mkdtemp(prefix="capsem-test-"))
        self.uds_path = self.tmp_dir / f"service-{uuid.uuid4().hex[:8]}.sock"
        self.proc = None
        self._log_file = None
        self.extra_env = extra_env or {}
        self.service_toml = service_toml
        self.pass_assets_dir = pass_assets_dir
        self.assets_dir = Path(assets_dir) if assets_dir is not None else None

    def start(self):
        # Sign binaries before spawning (macOS needs virtualization entitlement)
        sign_binary(PROCESS_BINARY)
        sign_binary(SERVICE_BINARY)
        sign_binary(GATEWAY_BINARY)
        sign_binary(TRAY_BINARY)

        arch = "arm64" if os.uname().machine == "arm64" else "x86_64"
        assets_dir = self.assets_dir or (ASSETS_DIR / arch)
        asset_cache = self.tmp_dir / "assets"

        env = os.environ.copy()
        env["RUST_LOG"] = "debug"
        env["CAPSEM_RUN_DIR"] = str(self.tmp_dir)
        env["CAPSEM_HOME"] = str(self.tmp_dir)
        env["HOME"] = str(self.tmp_dir)
        env.update(self.extra_env)

        log_path = self.tmp_dir / "service.log"
        print(f"SERVICE LOG: {log_path}")

        if self.service_toml is not None:
            (self.tmp_dir / "service.toml").write_text(self.service_toml)
        else:
            assets = {
                "vmlinuz": find_asset(assets_dir, "vmlinuz"),
                "initrd.img": find_asset(assets_dir, "initrd.img"),
                "rootfs.squashfs": find_asset(assets_dir, "rootfs.squashfs"),
            }
            write_profile_home(self.tmp_dir, asset_cache, assets)
            assets_dir = asset_cache

        # Deliberately omit --tray-binary: the tray is a user-facing macOS
        # menu bar icon and spawning it on every test instance flashes the
        # menu bar dozens of times during a full suite run. Companion
        # lifecycle tests exercise the tray via their own spawn.
        cmd = [
            str(SERVICE_BINARY),
            "--uds-path",
            str(self.uds_path),
            "--process-binary",
            str(PROCESS_BINARY),
            "--gateway-binary",
            str(GATEWAY_BINARY),
            "--gateway-port",
            "0",
            "--parent-pid",
            str(os.getpid()),
            "--foreground",
        ]
        if self.pass_assets_dir:
            cmd += ["--assets-dir", str(assets_dir)]

        log_fd = os.open(log_path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o644)
        try:
            self.proc = subprocess.Popen(
                cmd,
                env=env,
                stdout=log_fd,
                stderr=log_fd,
            )
        finally:
            os.close(log_fd)

        start = time.time()
        while time.time() - start < 15:
            if self.proc.poll() is not None:
                code = self.proc.returncode
                log_text = log_path.read_text() if log_path.exists() else ""
                self.stop()
                if log_text:
                    print(f"\n--- SERVICE LOG ---\n{log_text}\n---", file=sys.stderr)
                raise RuntimeError(
                    f"capsem-service exited before accepting connections (exit={code})"
                )
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

        log_text = log_path.read_text() if log_path.exists() else ""
        self.stop()
        if log_text:
            print(f"\n--- SERVICE LOG ---\n{log_text}\n---", file=sys.stderr)
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


def select_editable_profile(client, source_profile="profile-asset-boot", prefix="pytest"):
    """Fork the locked E2E profile and select the fork for mutation tests."""
    profile_id = f"{prefix}-{uuid.uuid4().hex[:8]}"
    created = client.post(
        f"/profiles/{source_profile}/fork",
        {"id": profile_id, "name": f"{prefix} editable profile"},
    )
    assert created is not None and created.get("profile", {}).get("id") == profile_id, (
        f"failed to fork editable profile: {created}"
    )
    selected = client.post(f"/settings/presets/{profile_id}", {})
    selected_default = (
        selected.get("settings_profiles", {}).get("service", {}).get("default_profile")
        if selected
        else None
    )
    assert selected is not None and selected_default == profile_id, (
        f"failed to select editable profile {profile_id}: {selected}"
    )
    return profile_id


def vm_name(prefix="test"):
    """Generate a unique VM name with the given prefix."""
    return f"{prefix}-{uuid.uuid4().hex[:8]}"
