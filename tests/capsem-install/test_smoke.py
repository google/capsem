"""Smoke tests for the install test harness.

These verify the test infrastructure works before any real install tests run.
If test_systemd_works fails, the entire suite should be investigated -- it
means the Docker container isn't set up correctly for systemd user sessions.
"""

from __future__ import annotations

import subprocess

import pytest


class TestHarnessSmoke:
    """Verify the test harness itself works."""

    def test_systemd_works(self, systemd_available):
        """systemctl --user status works in this container.

        If this fails, all systemd-dependent tests will also fail.
        Fix the Docker setup before investigating other failures.
        """
        result = subprocess.run(
            ["systemctl", "--user", "status"],
            capture_output=True,
            text=True,
        )
        # 0 = running units, 3 = no units loaded (both are fine)
        assert result.returncode in (0, 3), (
            f"systemctl --user status failed (rc={result.returncode}):\n{result.stderr}"
        )

    def test_installed_layout_has_binaries(self, installed_layout):
        """All 6 binaries are present after simulate-install.sh."""
        from conftest import BINARIES, INSTALL_DIR

        for name in BINARIES:
            assert (INSTALL_DIR / name).exists(), f"missing: {name}"

    def test_capsem_version_has_build_hash(self, installed_layout):
        """capsem version includes the build hash."""
        from conftest import get_build_hash

        build_hash = get_build_hash()
        assert "." in build_hash, f"build hash should contain '.': {build_hash}"
        # Format: <git-short-sha>.<timestamp>
        parts = build_hash.split(".")
        assert len(parts) == 2, f"expected <sha>.<ts>, got: {build_hash}"
        assert len(parts[0]) >= 7, f"git SHA too short: {parts[0]}"
        assert parts[1].isdigit(), f"timestamp not numeric: {parts[1]}"
