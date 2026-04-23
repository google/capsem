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
import os
import re
import shutil
import signal
import subprocess
import tempfile
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
    ``~/.capsem`` -- `test_full_uninstall` literally asserts it got removed,
    and `simulate-install.sh` would overwrite binaries the developer had
    just installed. We isolate under a dedicated temp dir so bare-metal
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


CAPSEM_DIR = _resolve_capsem_home()
INSTALL_DIR = CAPSEM_DIR / "bin"
ASSETS_DIR = CAPSEM_DIR / "assets"
RUN_DIR = CAPSEM_DIR / "run"

BINARIES = ["capsem", "capsem-service", "capsem-process", "capsem-mcp", "capsem-gateway", "capsem-tray"]
DEFAULT_TIMEOUT = 30


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


def _ensure_installed() -> None:
    """(Re)run simulate-install.sh if any expected binary is missing."""
    if os.environ.get("CAPSEM_DEB_INSTALLED") == "1":
        for name in BINARIES:
            binary = INSTALL_DIR / name
            assert binary.exists(), f"binary not installed by dpkg: {binary}"
            assert os.access(binary, os.X_OK), f"binary not executable: {binary}"
        return

    if all((INSTALL_DIR / name).exists() for name in BINARIES):
        return

    bin_src = os.environ.get("CAPSEM_BIN_SRC", "target/debug")
    assets_src = os.environ.get("CAPSEM_ASSETS_SRC", "assets")
    script = Path(__file__).parent.parent.parent / "scripts" / "simulate-install.sh"
    assert script.exists(), f"simulate-install.sh not found at {script}"
    result = subprocess.run(
        ["bash", str(script), bin_src, assets_src],
        capture_output=True, text=True, timeout=60,
    )
    assert result.returncode == 0, (
        f"simulate-install.sh failed:\nstdout: {result.stdout}\nstderr: {result.stderr}"
    )
    for name in BINARIES:
        binary = INSTALL_DIR / name
        assert binary.exists(), f"binary not installed: {binary}"
        assert os.access(binary, os.X_OK), f"binary not executable: {binary}"


@pytest.fixture
def installed_layout() -> Path:
    """Self-healing installed layout fixture.

    Function-scoped (not session-scoped) so destructive tests like
    test_full_uninstall don't poison the rest of the suite. Re-runs
    simulate-install.sh only when binaries are missing, so the per-test
    overhead is one stat() per binary in the common case.
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
