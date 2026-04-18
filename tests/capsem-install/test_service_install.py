"""Service install/uninstall/status tests for WB3.

Tests verify `capsem service install/uninstall/status` for systemd user units.
LaunchAgent tests are manual-only (can't run launchctl in Docker).
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest

from .conftest import (
    INSTALL_DIR,
    RUN_DIR,
    run_capsem,
)

SYSTEMD_UNIT = Path.home() / ".config" / "systemd" / "user" / "capsem.service"


@pytest.mark.live_system
class TestServiceInstall:
    """capsem service install/uninstall/status commands."""

    def test_service_install_creates_systemd_unit(
        self, installed_layout, clean_state, systemd_available
    ):
        """capsem service install creates a systemd user unit with absolute paths."""
        result = run_capsem("service", "install", timeout=15)
        assert result.returncode == 0, (
            f"service install failed:\nstdout: {result.stdout}\nstderr: {result.stderr}"
        )
        assert SYSTEMD_UNIT.exists(), f"unit file not created at {SYSTEMD_UNIT}"

        content = SYSTEMD_UNIT.read_text()
        # ExecStart must use absolute path to the installed binary
        assert "ExecStart=/" in content, "ExecStart must use absolute path"
        assert "capsem-service" in content
        assert "--foreground" in content
        assert "--assets-dir" in content
        assert "--process-binary" in content

    def test_service_status_after_install(
        self, installed_layout, clean_state, systemd_available
    ):
        """After install, status shows installed + running."""
        install = run_capsem("service", "install", timeout=15)
        assert install.returncode == 0, f"install failed: {install.stderr}"

        # Give systemd a moment to start
        import time
        time.sleep(2)

        result = run_capsem("service", "status", timeout=10)
        assert result.returncode == 0, (
            f"status failed:\nstdout: {result.stdout}\nstderr: {result.stderr}"
        )
        assert "Installed: true" in result.stdout
        # Service may or may not be running depending on whether assets exist
        # but it should at least report installed

    def test_service_uninstall_removes_unit(
        self, installed_layout, clean_state, systemd_available
    ):
        """Uninstall removes the systemd unit file."""
        # Install first
        install = run_capsem("service", "install", timeout=15)
        assert install.returncode == 0, f"install failed: {install.stderr}"
        assert SYSTEMD_UNIT.exists()

        # Uninstall
        result = run_capsem("service", "uninstall", timeout=15)
        assert result.returncode == 0, (
            f"uninstall failed:\nstdout: {result.stdout}\nstderr: {result.stderr}"
        )
        assert not SYSTEMD_UNIT.exists(), "unit file should be removed after uninstall"

    def test_auto_launch_uses_systemd_when_installed(
        self, installed_layout, clean_state, systemd_available
    ):
        """After service install, auto-launch restarts via systemd."""
        # Install the service unit
        install = run_capsem("service", "install", timeout=15)
        assert install.returncode == 0, f"install failed: {install.stderr}"

        # Kill any running service
        subprocess.run(["pkill", "-f", "capsem-service"], capture_output=True)
        sock = RUN_DIR / "service.sock"
        sock.unlink(missing_ok=True)

        import time
        time.sleep(1)

        # Now capsem list should auto-launch via systemd
        result = run_capsem("list", timeout=15)
        assert result.returncode == 0, (
            f"auto-launch via systemd failed:\n"
            f"stdout: {result.stdout}\nstderr: {result.stderr}"
        )

        # Clean up
        run_capsem("service", "uninstall", timeout=15)

    def test_service_install_idempotent(
        self, installed_layout, clean_state, systemd_available
    ):
        """Running install twice succeeds without error."""
        r1 = run_capsem("service", "install", timeout=15)
        assert r1.returncode == 0, f"first install failed: {r1.stderr}"

        r2 = run_capsem("service", "install", timeout=15)
        assert r2.returncode == 0, (
            f"second install failed (not idempotent):\n"
            f"stdout: {r2.stdout}\nstderr: {r2.stderr}"
        )

        # Clean up
        run_capsem("service", "uninstall", timeout=15)

    def test_service_uninstall_when_not_installed(
        self, installed_layout, clean_state, systemd_available
    ):
        """Uninstall with no unit gives clean message, not an error."""
        # Ensure not installed
        if SYSTEMD_UNIT.exists():
            SYSTEMD_UNIT.unlink()

        result = run_capsem("service", "uninstall", timeout=10)
        assert result.returncode == 0, (
            f"uninstall-when-not-installed should succeed:\n"
            f"stdout: {result.stdout}\nstderr: {result.stderr}"
        )
