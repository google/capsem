"""Shared fixtures for capsem install e2e tests.

Tests are split into two tiers:
  - **packaging**: verify the installed layout, binaries, CLI commands that
    don't need a running service. Run in Docker during `just test-install`.
  - **live_system**: need a running service with VM assets (kernel, rootfs).
    Only run on real systems (macOS or Linux with assets). Marked with
    @pytest.mark.live_system and skipped when CAPSEM_DEB_INSTALLED=1.
"""

from __future__ import annotations

import atexit
import filecmp
import json
import os
import platform
import re
import shutil
import signal
import subprocess
import tempfile
import time
from pathlib import Path

import pytest


def pytest_configure(config):
    config.addinivalue_line(
        "markers",
        "live_system: test requires a running service with VM assets (skipped in packaging tests)",
    )


def pytest_collection_modifyitems(config, items):
    reason = None
    if os.environ.get("CAPSEM_DEB_INSTALLED") == "1":
        reason = "live_system test skipped in Docker packaging harness (no VM assets)"
    elif not os.environ.get("CAPSEM_ALLOW_DESTRUCTIVE"):
        # live_system tests call `capsem setup` / `capsem uninstall`, which
        # write to ~/Library/LaunchAgents/com.capsem.service.plist or
        # ~/.config/systemd/user/capsem.service -- these paths can't be
        # redirected by CAPSEM_HOME, so running them bare-metal would overwrite
        # the developer's actual installed service. Opt in with
        # CAPSEM_ALLOW_DESTRUCTIVE=1.
        reason = (
            "live_system test skipped to avoid mutating real LaunchAgent / "
            "systemd unit; set CAPSEM_ALLOW_DESTRUCTIVE=1 to opt in"
        )
    if reason is None:
        return
    skip = pytest.mark.skip(reason=reason)
    for item in items:
        if "live_system" in item.keywords:
            item.add_marker(skip)


def _resolve_capsem_home() -> Path:
    """Pick the CAPSEM_HOME these tests operate on.

    Running bare-metal, the suite used to clobber the developer's real
    ``~/.capsem`` -- uninstall tests and `simulate-install.sh` mutate the
    installed runtime. We isolate under a dedicated temp dir so bare-metal
    `pytest tests/capsem-install/` is always safe.

    Exceptions:
      - ``CAPSEM_DEB_INSTALLED=1`` (Docker install-test harness): we are
        the system under test -- write to the real $HOME/.capsem there.
      - ``CAPSEM_HOME`` already set by the caller: honor it (lets `just
        test` point the suite at its isolated ``target/test-home``).
    """
    if os.environ.get("CAPSEM_DEB_INSTALLED") == "1":
        return Path.home() / ".capsem"
    env = os.environ.get("CAPSEM_HOME")
    if env:
        return Path(env)
    d = Path(tempfile.mkdtemp(prefix="capsem-install-test-"))
    os.environ["CAPSEM_HOME"] = str(d)
    # Mirror the env override the Rust helpers honor, so child processes
    # (capsem + simulate-install.sh) write into the same isolated tree.
    os.environ["CAPSEM_RUN_DIR"] = str(d / "run")
    atexit.register(lambda p=d: shutil.rmtree(p, ignore_errors=True))
    return d


PROJECT_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_BIN_SRC = PROJECT_ROOT / "target" / "debug"
HOST_CRATES = [
    "capsem-service",
    "capsem-process",
    "capsem",
    "capsem-mcp",
    "capsem-mcp-aggregator",
    "capsem-mcp-builtin",
    "capsem-gateway",
    "capsem-tray",
]

CAPSEM_DIR = _resolve_capsem_home()
INSTALL_DIR = CAPSEM_DIR / "bin"
ASSETS_DIR = CAPSEM_DIR / "assets"
RUN_DIR = CAPSEM_DIR / "run"

BINARIES = [
    "capsem",
    "capsem-service",
    "capsem-process",
    "capsem-mcp",
    "capsem-mcp-aggregator",
    "capsem-mcp-builtin",
    "capsem-gateway",
    "capsem-tray",
]
DEFAULT_TIMEOUT = 30
_LOCAL_BUILD_DONE = False


def run_capsem(*args: str, timeout: int = DEFAULT_TIMEOUT) -> subprocess.CompletedProcess[str]:
    """Run the installed capsem binary with capture + timeout."""
    return subprocess.run(
        [str(INSTALL_DIR / "capsem"), *args],
        capture_output=True,
        text=True,
        timeout=timeout,
    )


def get_build_hash() -> str:
    """Run capsem version and parse the build hash from '(build ...)'."""
    r = run_capsem("version")
    assert r.returncode == 0, f"capsem version failed: {r.stderr}"
    match = re.search(r"\(build ([^)]+)\)", r.stdout)
    assert match, f"no build hash in version output: {r.stdout}"
    return match.group(1)


def _kill_service() -> None:
    """Kill any running capsem-service and companion processes from the
    installed layout at ``~/.capsem/bin/``.

    Scoped to the installed prefix so it never reaches into parallel test
    workers running ``target/debug/capsem-service``. A broad ``pkill -f
    capsem-service`` would race with every other pytest worker on the box,
    which was the original cascade that poisoned the full suite.
    """
    pidfile = RUN_DIR / "service.pid"
    if pidfile.exists():
        try:
            pid = int(pidfile.read_text().strip())
            os.kill(pid, signal.SIGTERM)
            
            # Bounded wait + SIGKILL fallback
            start = time.time()
            while time.time() - start < 5:
                try:
                    os.kill(pid, 0)
                except ProcessLookupError:
                    break
                time.sleep(0.2)
            else:
                try:
                    os.kill(pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
        except (ValueError, ProcessLookupError, PermissionError):
            pass
        pidfile.unlink(missing_ok=True)

    # Fallback: only kill processes whose executable path lives under the
    # installed prefix. We build the pattern from INSTALL_DIR so HOME expansion
    # is consistent and we never match target/debug binaries.
    install_prefix = str(INSTALL_DIR) + "/"
    for proc_name in ["capsem-service", "capsem-gateway", "capsem-tray", "capsem-process"]:
        subprocess.run(
            ["pkill", "-f", f"{install_prefix}{proc_name}"],
            capture_output=True,
        )

    # Remove stale socket
    sock = RUN_DIR / "service.sock"
    sock.unlink(missing_ok=True)


def _binary_is_current(name: str, bin_src: Path, install_dir: Path) -> bool:
    """Return true when the simulated installed binary matches its source."""
    src = bin_src / name
    dst = install_dir / name
    if not src.is_file() or not dst.is_file():
        return False
    return filecmp.cmp(src, dst, shallow=False)


def _installed_binaries_current(bin_src: Path, install_dir: Path) -> bool:
    return all(_binary_is_current(name, bin_src, install_dir) for name in BINARIES)


def _resolve_bin_src() -> Path:
    env_src = os.environ.get("CAPSEM_BIN_SRC")
    if env_src:
        path = Path(env_src).expanduser()
    elif os.environ.get("CAPSEM_DEB_INSTALLED") == "1":
        # The Docker packaging harness builds host crates with
        # CARGO_TARGET_DIR=/cargo-target. Using /src/target/debug here can pick
        # up macOS host binaries from the bind mount and trigger Exec format
        # errors when simulate-install refreshes the fixture.
        cargo_target_debug = Path("/cargo-target/debug")
        if cargo_target_debug.exists():
            path = cargo_target_debug
        else:
            path = DEFAULT_BIN_SRC
    else:
        path = DEFAULT_BIN_SRC

    if not path.is_absolute():
        path = PROJECT_ROOT / path
    return path


def _resolve_assets_src() -> Path:
    path = Path(os.environ.get("CAPSEM_ASSETS_SRC", "assets")).expanduser()
    if not path.is_absolute():
        path = PROJECT_ROOT / path
    return path


def _should_build_default_bin_src(bin_src: Path) -> bool:
    if os.environ.get("CAPSEM_INSTALL_SKIP_BUILD") == "1":
        return False
    try:
        return bin_src.resolve() == DEFAULT_BIN_SRC.resolve()
    except FileNotFoundError:
        return bin_src.absolute() == DEFAULT_BIN_SRC.absolute()


def _ensure_local_binaries_built(bin_src: Path) -> None:
    global _LOCAL_BUILD_DONE
    if _LOCAL_BUILD_DONE or not _should_build_default_bin_src(bin_src):
        return

    cmd = ["cargo", "build"]
    for crate in HOST_CRATES:
        cmd.extend(["-p", crate])
    result = subprocess.run(
        cmd,
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        timeout=600,
    )
    assert result.returncode == 0, (
        f"cargo build for install-test binaries failed:\n"
        f"stdout: {result.stdout}\nstderr: {result.stderr}"
    )
    _LOCAL_BUILD_DONE = True


def _ensure_installed() -> None:
    """Build and (re)run simulate-install.sh when installed binaries are stale."""
    bin_src = _resolve_bin_src()
    assets_src = _resolve_assets_src()

    if os.environ.get("CAPSEM_DEB_INSTALLED") == "1":
        for name in BINARIES:
            binary = INSTALL_DIR / name
            assert binary.exists(), f"binary not installed by dpkg: {binary}"
            assert os.access(binary, os.X_OK), f"binary not executable: {binary}"
        if _installed_binaries_current(bin_src, INSTALL_DIR) and _installed_assets_ready():
            return
    else:
        _ensure_local_binaries_built(bin_src)
        if _installed_binaries_current(bin_src, INSTALL_DIR):
            return

    if not bin_src.exists():
        raise AssertionError(
            f"capsem binary source directory not found: {bin_src}\n"
            "Set CAPSEM_BIN_SRC to a valid built binary directory."
        )
    if not assets_src.exists():
        raise AssertionError(
            f"capsem assets source directory not found: {assets_src}\n"
            "Set CAPSEM_ASSETS_SRC to a valid assets directory."
        )

    script = PROJECT_ROOT / "scripts" / "simulate-install.sh"
    assert script.exists(), f"simulate-install.sh not found at {script}"
    result = subprocess.run(
        ["bash", str(script), str(bin_src), str(assets_src)],
        capture_output=True, text=True, timeout=60,
    )
    assert result.returncode == 0, (
        f"simulate-install.sh failed:\nstdout: {result.stdout}\nstderr: {result.stderr}"
    )
    for name in BINARIES:
        binary = INSTALL_DIR / name
        assert binary.exists(), f"binary not installed: {binary}"
        assert os.access(binary, os.X_OK), f"binary not executable: {binary}"


def _installed_assets_ready() -> bool:
    manifest = ASSETS_DIR / "manifest.json"
    if not manifest.is_file():
        return False

    try:
        data = json.loads(manifest.read_text(encoding="utf-8"))
        current = data["assets"]["current"]
        arch = _host_manifest_arch()
        arch_assets = data["assets"]["releases"][current]["arches"][arch]
    except (OSError, ValueError, KeyError, TypeError):
        return False

    for logical in ("vmlinuz", "initrd.img", "rootfs.squashfs"):
        entry = arch_assets.get(logical)
        if not isinstance(entry, dict):
            return False
        h = entry.get("hash")
        if not isinstance(h, str) or len(h) < 16:
            return False
        stem, dot, suffix = logical.partition(".")
        hashed = f"{stem}-{h[:16]}"
        if dot:
            hashed += f".{suffix}"
        if not (ASSETS_DIR / arch / hashed).is_file():
            return False
    return True


def _host_manifest_arch() -> str:
    machine = platform.machine().lower()
    if machine in ("aarch64", "arm64"):
        return "arm64"
    return "x86_64"


@pytest.fixture
def installed_layout() -> Path:
    """Self-healing installed layout fixture.

    Function-scoped (not session-scoped) so destructive tests like
    uninstall tests don't poison the rest of the suite. Local runs build the
    default host binaries once, then re-run simulate-install.sh when binaries
    are missing or differ from CAPSEM_BIN_SRC so tests cannot silently use
    stale helpers.
    """
    _ensure_installed()
    return INSTALL_DIR


@pytest.fixture
def clean_state():
    """Kill any running service, clear run dir, yield, kill again."""
    _kill_service()
    # Clear run dir but keep the directory
    if RUN_DIR.exists():
        for f in RUN_DIR.iterdir():
            if f.is_dir():
                shutil.rmtree(f, ignore_errors=True)
            else:
                f.unlink(missing_ok=True)
    yield
    _kill_service()


@pytest.fixture
def systemd_available():
    """Check if systemd user session is available. Skip if not (e.g. macOS)."""
    try:
        result = subprocess.run(
            ["systemctl", "--user", "status"],
            capture_output=True,
            text=True,
        )
    except FileNotFoundError:
        pytest.skip("systemctl not installed (non-systemd OS)")
    if result.returncode not in (0, 3):  # 3 = no units loaded, still works
        pytest.skip("systemd user session not available")
