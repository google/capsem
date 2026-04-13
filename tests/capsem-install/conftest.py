"""Shared fixtures for capsem install e2e tests.

All tests depend on the `installed_layout` fixture which exercises the real
install flow via simulate-install.sh. Tests run as the capsem user inside
a Docker container with systemd as PID 1.
"""

from __future__ import annotations

import os
import re
import shutil
import signal
import subprocess
from pathlib import Path

import pytest

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
    """Kill any running capsem-service and companion processes."""
    pidfile = RUN_DIR / "service.pid"
    if pidfile.exists():
        try:
            pid = int(pidfile.read_text().strip())
            os.kill(pid, signal.SIGTERM)
        except (ValueError, ProcessLookupError, PermissionError):
            pass
        pidfile.unlink(missing_ok=True)

    # Also kill by name as fallback (service + companions)
    for proc_name in ["capsem-service", "capsem-gateway", "capsem-tray", "capsem-process"]:
        subprocess.run(
            ["pkill", "-f", proc_name],
            capture_output=True,
        )

    # Remove stale socket
    sock = RUN_DIR / "service.sock"
    sock.unlink(missing_ok=True)


@pytest.fixture(scope="session")
def installed_layout() -> Path:
    """Install capsem binaries via simulate-install.sh.

    Session-scoped: runs once, all tests share the installed layout.
    Asserts all 6 binaries and the install directory exist.
    """
    # Find the source directories -- in Docker these are under /src/target/debug
    # and /src/assets respectively. Locally they may vary.
    bin_src = os.environ.get("CAPSEM_BIN_SRC", "target/debug")
    assets_src = os.environ.get("CAPSEM_ASSETS_SRC", "assets")

    script = Path(__file__).parent.parent.parent / "scripts" / "simulate-install.sh"
    assert script.exists(), f"simulate-install.sh not found at {script}"

    result = subprocess.run(
        ["bash", str(script), bin_src, assets_src],
        capture_output=True,
        text=True,
        timeout=60,
    )
    assert result.returncode == 0, (
        f"simulate-install.sh failed:\nstdout: {result.stdout}\nstderr: {result.stderr}"
    )

    # Verify all binaries exist
    for name in BINARIES:
        binary = INSTALL_DIR / name
        assert binary.exists(), f"binary not installed: {binary}"
        assert os.access(binary, os.X_OK), f"binary not executable: {binary}"

    return INSTALL_DIR


@pytest.fixture
def clean_state():
    """Kill any running service, clear run dir, yield, kill again."""
    _kill_service()
    # Clear run dir but keep the directory
    if RUN_DIR.exists():
        for f in RUN_DIR.iterdir():
            f.unlink(missing_ok=True)
    yield
    _kill_service()


@pytest.fixture
def systemd_available():
    """Check if systemd user session is available. Skip if not."""
    result = subprocess.run(
        ["systemctl", "--user", "status"],
        capture_output=True,
        text=True,
    )
    if result.returncode not in (0, 3):  # 3 = no units loaded, still works
        pytest.skip("systemd user session not available")
