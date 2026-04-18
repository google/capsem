"""Auto-launch tests for WB1: CLI auto-launches service on first command.

Tests verify that `capsem list` (or any command) works without manually
starting the service. The installed layout is exercised via simulate-install.sh.
"""

from __future__ import annotations

import os
import signal
import stat
import subprocess
import time
from pathlib import Path

import pytest

from .conftest import (
    INSTALL_DIR,
    RUN_DIR,
    run_capsem,
    BINARIES,
)


class TestAutoLaunch:
    """Service auto-launches when CLI connects and no service is running."""

    @pytest.mark.live_system
    def test_auto_launch_from_installed_layout(self, installed_layout, clean_state):
        """capsem list auto-starts service when socket is absent."""
        # Ensure no service running and no socket
        sock = RUN_DIR / "service.sock"
        assert not sock.exists(), "stale socket should be cleaned"

        result = run_capsem("list", timeout=15)
        # Service should have started and responded
        assert result.returncode == 0, (
            f"capsem list failed (rc={result.returncode}):\n"
            f"stdout: {result.stdout}\nstderr: {result.stderr}"
        )

    def test_path_discovery_installed_layout(self, installed_layout):
        """All sibling binaries are discoverable from the installed capsem."""
        # capsem binary should find capsem-service, capsem-process in same dir
        for name in BINARIES:
            binary = INSTALL_DIR / name
            assert binary.exists(), f"sibling binary not found: {name}"
            assert os.access(binary, os.X_OK), f"not executable: {name}"

    @pytest.mark.live_system
    def test_asset_resolution_installed_layout(self, installed_layout, clean_state):
        """Service finds assets at ~/.capsem/assets/ in installed layout."""
        # If assets were installed, service should start without --assets-dir
        # We test this indirectly: capsem list triggers auto-launch which
        # must resolve assets to pass them to the service
        result = run_capsem("list", timeout=15)
        assert result.returncode == 0, (
            f"asset resolution failed:\nstdout: {result.stdout}\nstderr: {result.stderr}"
        )

    def test_auto_launch_bad_service_binary(self, installed_layout, clean_state):
        """Clear error when capsem-service binary is broken (not a hang)."""
        service_bin = INSTALL_DIR / "capsem-service"
        original = service_bin.read_bytes()

        try:
            # Replace service binary with a stub that exits immediately with error
            service_bin.write_text("#!/bin/sh\nexit 1\n")
            service_bin.chmod(stat.S_IRWXU | stat.S_IRGRP | stat.S_IXGRP | stat.S_IROTH | stat.S_IXOTH)

            result = run_capsem("list", timeout=15)
            # Should fail with an error, not hang
            assert result.returncode != 0, "should fail with broken service binary"
            combined = result.stdout + result.stderr
            assert "failed" in combined.lower() or "error" in combined.lower(), (
                f"expected error message, got:\nstdout: {result.stdout}\nstderr: {result.stderr}"
            )
        finally:
            # Restore original binary
            service_bin.write_bytes(original)
            service_bin.chmod(stat.S_IRWXU | stat.S_IRGRP | stat.S_IXGRP | stat.S_IROTH | stat.S_IXOTH)

    @pytest.mark.live_system
    def test_auto_launch_missing_assets(self, installed_layout, clean_state):
        """Clear error when assets directory is empty or missing."""
        from conftest import ASSETS_DIR

        # Temporarily rename assets dir
        backup = ASSETS_DIR.parent / "assets_backup"
        moved = False
        try:
            if ASSETS_DIR.exists():
                ASSETS_DIR.rename(backup)
                moved = True

            result = run_capsem("list", timeout=15)
            # Should fail -- cannot resolve assets
            assert result.returncode != 0, "should fail with missing assets"
            combined = result.stdout + result.stderr
            assert "assets" in combined.lower() or "error" in combined.lower(), (
                f"expected assets error, got:\nstdout: {result.stdout}\nstderr: {result.stderr}"
            )
        finally:
            if moved and backup.exists():
                backup.rename(ASSETS_DIR)
