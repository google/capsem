"""Shared fixtures for capsem install e2e tests.

Tests are split into two tiers:
  - **packaging**: verify the installed layout, binaries, CLI commands that
    don't need a running service. Run in Docker during `just test-install`.
  - **live_system**: need a running service with VM assets (kernel, rootfs).
    Only run on real systems (macOS or Linux with assets). Marked with
    @pytest.mark.live_system and skipped when CAPSEM_DEB_INSTALLED=1.
"""

from __future__ import annotations

import os
import re
import shutil
import signal
import subprocess
from pathlib import Path

import pytest


def pytest_configure(config):
    config.addinivalue_line(
        "markers",
        "live_system: test requires a running service with VM assets (skipped in packaging tests)",
    )


def pytest_collection_modifyitems(config, items):
    if os.environ.get("CAPSEM_DEB_INSTALLED") == "1":
        skip = pytest.mark.skip(reason="live_system test -- requires VM assets, skipped in packaging test")
        for item in items:
            if "live_system" in item.keywords:
                item.add_marker(skip)

INSTALL_DIR = Path.home() / ".capsem" / "bin"
ASSETS_DIR = Path.home() / ".capsem" / "assets"
RUN_DIR = Path.home() / ".capsem" / "run"
CAPSEM_DIR = Path.home() / ".capsem"

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
