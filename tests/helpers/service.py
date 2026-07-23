"""Shared service startup helper for integration tests."""

import os
import shutil
import subprocess
import sys
import tempfile
import time
import tomllib
import uuid

from pathlib import Path

from .constants import EXEC_READY_TIMEOUT
from .sign import sign_binary
from .uds_client import UdsHttpClient

PROJECT_ROOT = Path(__file__).parent.parent.parent
SERVICE_BINARY = PROJECT_ROOT / "target/debug/capsem-service"
PROCESS_BINARY = PROJECT_ROOT / "target/debug/capsem-process"
GATEWAY_BINARY = PROJECT_ROOT / "target/debug/capsem-gateway"
TRAY_BINARY = PROJECT_ROOT / "target/debug/capsem-tray"
ASSETS_DIR = PROJECT_ROOT / "assets"
PROFILES_DIR = PROJECT_ROOT / "target" / "config" / "profiles"
LINUX_TEST_TMP_PARENT = Path("/var/tmp/capsem-tests")


with (PROJECT_ROOT / "config" / "storage-policy.toml").open("rb") as _policy_stream:
    _DEBUG_ARTIFACT_POLICY = tomllib.load(_policy_stream)["debug_artifacts"]

ARTIFACT_MAX_FILE_BYTES = int(_DEBUG_ARTIFACT_POLICY["maximum_file_mib"]) * 1024 * 1024
ARTIFACT_SKIP_NAMES = frozenset(_DEBUG_ARTIFACT_POLICY["skip_names"]) | frozenset({
    # Multi-GB VM disk images -- regenerable from the build, would burn
    # disk at ~2 GB per failure and we've been there.
    "rootfs.img",
    "rootfs.img.backing",
    # VM memory checkpoints -- ~100MB+ per suspend, skip to save space.
    # The logs in the same directory are what we need for debugging.
    "checkpoint.vzsave",
})
ARTIFACT_MIN_KEPT_DIRS = int(_DEBUG_ARTIFACT_POLICY["minimum_runs"])
ARTIFACT_MAX_KEPT_DIRS = int(_DEBUG_ARTIFACT_POLICY["maximum_runs"])
ARTIFACT_MAX_AGE_S = int(_DEBUG_ARTIFACT_POLICY["maximum_age_days"]) * 24 * 60 * 60
ARTIFACT_MAX_TOTAL_BYTES = int(_DEBUG_ARTIFACT_POLICY["maximum_total_gib"]) * 1024**3


def capsem_test_tmp_parent() -> Path:
    """Return the parent directory for heavyweight integration-test scratch.

    Linux CI/dev containers often mount /tmp as tmpfs. Live VM fixtures can
    create multi-GB overlays, so default Linux test scratch to /var/tmp while
    keeping paths short enough for Unix-domain sockets.
    """
    configured = os.environ.get("CAPSEM_TEST_TMPDIR")
    parent = Path(configured) if configured else (
        LINUX_TEST_TMP_PARENT if sys.platform.startswith("linux") else Path(tempfile.gettempdir())
    )
    parent.mkdir(parents=True, exist_ok=True)
    return parent


def make_capsem_tmp_dir(prefix: str) -> Path:
    return Path(tempfile.mkdtemp(prefix=prefix, dir=capsem_test_tmp_parent()))


def make_service_home_run_dirs() -> tuple[Path, Path]:
    """Create the installed home/run layout for an isolated test service."""
    home_dir = make_capsem_tmp_dir("capsem-test-")
    run_dir = home_dir / "run"
    run_dir.mkdir()
    return home_dir, run_dir


def _contains_profile_toml(profiles_dir: Path) -> bool:
    return any(path.name == "profile.toml" for path in profiles_dir.glob("*/profile.toml"))


def materialize_test_profiles(tmp_dir: Path) -> Path:
    """Copy generated runtime profiles into a test run directory.

    Checked-in profiles are source contracts and intentionally do not contain
    asset hashes. VM-booting tests must use the materialized profiles generated
    under target/config/profiles, matching the service/runtime rail.
    """
    profiles_dir = tmp_dir / "config" / "profiles"
    if profiles_dir.exists():
        if not _contains_profile_toml(profiles_dir):
            raise RuntimeError(
                f"generated profile directory contains no profile.toml: {profiles_dir}. "
                "Run `just _materialize-config` or a just recipe that depends on it."
            )
        return profiles_dir
    if not PROFILES_DIR.exists():
        raise RuntimeError(
            f"generated profile directory missing: {PROFILES_DIR}. "
            "Run `just _materialize-config` or a just recipe that depends on it."
        )
    if not _contains_profile_toml(PROFILES_DIR):
        raise RuntimeError(
            f"generated profile directory contains no profile.toml: {PROFILES_DIR}. "
            "Run `just _materialize-config` or a just recipe that depends on it."
        )
    shutil.copytree(PROFILES_DIR, profiles_dir)
    return profiles_dir


def preserve_tmp_dir_on_failure(tmp_dir, *, force: bool = False):
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
    force = force or bool(os.environ.get("CAPSEM_TEST_PRESERVE_ALWAYS"))
    current_test = os.environ.get("PYTEST_CURRENT_TEST", "").rsplit(" (", 1)[0]
    if not force:
        if current_test and current_test not in FAILED_NODEIDS:
            return
        if not FAILED_NODEIDS:
            return
    import stat as statmod
    import time
    worker = os.environ.get("PYTEST_XDIST_WORKER", "master")
    if current_test:
        tag = current_test.replace("/", "_").replace(":", "_")[:80]
    elif FAILED_NODEIDS:
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
        _rotate_artifacts(
            ARTIFACTS_ROOT,
            keep=ARTIFACT_MAX_KEPT_DIRS,
            minimum=ARTIFACT_MIN_KEPT_DIRS,
            maximum_age_s=ARTIFACT_MAX_AGE_S,
            maximum_total_bytes=ARTIFACT_MAX_TOTAL_BYTES,
        )
    except Exception as e:
        print(f"ARTIFACT: preserve fatal for {tmp_dir}: {e}", file=sys.stderr)


def _artifact_tree_size(path: Path) -> int:
    total = 0
    for candidate in path.rglob("*"):
        try:
            if candidate.is_file():
                total += candidate.stat().st_size
        except OSError:
            continue
    return total


def _rotate_artifacts(root, keep, minimum, maximum_age_s, maximum_total_bytes):
    """Bound failure evidence while always retaining the newest minimum."""
    if not root.exists():
        return
    try:
        dirs = sorted(
            (p for p in root.iterdir() if p.is_dir()),
            key=lambda p: p.name,  # names begin with YYYYMMDD-HHMMSS so string sort == chronological
        )
        protected = set(dirs[-minimum:]) if minimum > 0 else set()
        now = time.time()
        stale_dirs = list(dirs[:-keep] if keep > 0 else dirs)
        stale_dirs.extend(
            path
            for path in dirs
            if path not in protected and now - path.stat().st_mtime > maximum_age_s
        )
        for stale in dict.fromkeys(stale_dirs):
            shutil.rmtree(stale, ignore_errors=True)
        dirs = [path for path in dirs if path.exists()]
        sizes = {path: _artifact_tree_size(path) for path in dirs}
        total = sum(sizes.values())
        remaining_count = len(dirs)
        for stale in dirs:
            if total <= maximum_total_bytes or remaining_count <= minimum:
                break
            if stale in protected:
                continue
            shutil.rmtree(stale, ignore_errors=True)
            total -= sizes[stale]
            remaining_count -= 1
    except OSError as e:
        print(f"ARTIFACT: rotation skipped: {e}", file=sys.stderr)


class ServiceInstance:
    """A running capsem-service instance on an isolated socket."""

    def __init__(self, *, assets_dir: Path | None = None):
        # Match the installed layout exactly: CAPSEM_HOME owns a run/
        # directory and sessions/main.db is its sibling.  Using the temporary
        # home itself as CAPSEM_RUN_DIR makes main_db_path_for_run_dir() resolve
        # to the shared system temporary directory, so parallel workers all
        # write the same /tmp/sessions/main.db.
        self.home_dir, self.tmp_dir = make_service_home_run_dirs()
        self.uds_path = self.tmp_dir / f"service-{uuid.uuid4().hex[:8]}.sock"
        self.assets_dir = assets_dir
        self.profiles_dir = None
        self.proc = None
        self._log_file = None

    def start(self):
        # Sign binaries before spawning (macOS needs virtualization entitlement)
        sign_binary(PROCESS_BINARY)
        sign_binary(SERVICE_BINARY)
        sign_binary(GATEWAY_BINARY)
        sign_binary(TRAY_BINARY)

        assets_dir = self.assets_dir or ASSETS_DIR
        if self.profiles_dir is None:
            self.profiles_dir = materialize_test_profiles(self.tmp_dir)
        if not self.profiles_dir.exists():
            raise RuntimeError(
                f"generated profile directory missing: {self.profiles_dir}. "
                "Run `just _materialize-config` or a just recipe that depends on it."
            )

        env = os.environ.copy()
        env["RUST_LOG"] = "debug"
        env["CAPSEM_RUN_DIR"] = str(self.tmp_dir)
        env["CAPSEM_HOME"] = str(self.home_dir)
        env["CAPSEM_PROFILES_DIR"] = str(self.profiles_dir)
        env["CAPSEM_CREDENTIAL_STORE_PATH"] = str(
            self.home_dir / "credential-store.json"
        )
        env["HOME"] = str(self.home_dir)

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
                "--parent-pid", str(os.getpid()),
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
                         "--max-time", "2", "http://localhost/vms/list"],
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

    def stop(self, *, cleanup: bool = True):
        """Stop the service and clean up temporary directory.

        Gives the service enough time for graceful shutdown to reap every
        per-VM capsem-process child (SIGTERM -> 500ms grace -> SIGKILL
        survivors). SIGKILL here would skip that cleanup and orphan VMs.
        Tests that need to inspect shutdown-flushed state may pass
        ``cleanup=False`` and call ``stop()`` again after their assertions.
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

        if not cleanup:
            return

        # Tests commonly stop the service from a ``finally`` block.  That
        # happens before pytest's makereport hook records FAILED_NODEIDS, so
        # use the actively-propagating exception as authoritative failure
        # evidence instead of deleting the only service/process logs.
        if sys.exc_info()[0] is not None:
            preserve_tmp_dir_on_failure(self.home_dir, force=True)
        else:
            preserve_tmp_dir_on_failure(self.home_dir)

        if self.home_dir.exists():
            shutil.rmtree(self.home_dir, ignore_errors=True)


def wait_exec_ready(client, vm_name, timeout=EXEC_READY_TIMEOUT):
    """Wait until a VM responds to exec.

    The server's handle_exec already polls internally for VM readiness,
    so a single call with adequate timeout is sufficient -- no client-side
    retry loop needed.
    """
    try:
        resp = client.post(
            f"/vms/{vm_name}/exec",
            {"command": "echo ready", "timeout_secs": timeout},
            timeout=timeout + 5,
        )
        return resp is not None and "ready" in resp.get("stdout", "")
    except Exception:
        return False


def vm_record(client, typed_id):
    """Return the /vms/list row for a route id or user-facing VM name."""
    listing = client.get("/vms/list")
    for row in listing.get("sandboxes", []):
        if row.get("id") == typed_id or row.get("name") == typed_id:
            return row
    raise AssertionError(f"VM {typed_id!r} not found in /vms/list: {listing!r}")


def vm_route_id(client, typed_id):
    """Resolve a user-facing VM name to the UUID route id."""
    return vm_record(client, typed_id)["id"]


def vm_session_dir(tmp_dir, client, typed_id, *, must_exist=True):
    """Return the canonical on-disk session dir for a VM.

    VM names are display labels. The service owns random UUID ids and stores
    session state under those ids, so tests must not derive paths from names.
    """
    row = vm_record(client, typed_id)
    route_id = row["id"]
    candidates = [
        Path(tmp_dir) / "persistent" / route_id,
        Path(tmp_dir) / "sessions" / route_id,
        # Transitional fallbacks make assertion errors readable if an older
        # fixture or preserved artifact still uses the retired layout.
        Path(tmp_dir) / "persistent" / str(typed_id),
        Path(tmp_dir) / "sessions" / str(typed_id),
    ]
    for candidate in candidates:
        if candidate.exists():
            return candidate
    if must_exist:
        raise AssertionError(f"session dir missing for {typed_id!r}: {candidates}")
    return candidates[0] if row.get("persistent") else candidates[1]


def vm_session_db_path(tmp_dir, client, typed_id, *, must_exist=True):
    db_path = vm_session_dir(tmp_dir, client, typed_id, must_exist=must_exist) / "session.db"
    if must_exist and not db_path.exists():
        raise AssertionError(f"session.db missing at {db_path}")
    return db_path


def vm_name(prefix="test"):
    """Generate a unique VM name with the given prefix."""
    return f"{prefix}-{uuid.uuid4().hex[:8]}"
