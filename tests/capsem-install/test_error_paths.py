"""Error path tests for failure scenarios.

Verifies that failure modes produce actionable error messages (not stack
traces or silent failures) and that the system degrades gracefully.
"""

from __future__ import annotations

import stat

import pytest

from .conftest import (
    CAPSEM_DIR,
    RUN_DIR,
    ASSETS_DIR,
    run_capsem,
    temporarily_replace_installed_binary,
)


class TestErrorPaths:
    """Failure scenarios with actionable error messages."""

    def test_bad_service_binary(self, installed_layout, clean_state):
        """Broken capsem-service gives error, not hang."""
        with temporarily_replace_installed_binary("capsem-service", b"#!/bin/sh\nexit 1\n"):
            result = run_capsem("list", timeout=15)
            assert result.returncode != 0
            combined = (result.stdout + result.stderr).lower()
            assert "error" in combined or "failed" in combined, (
                f"expected error message: {result.stdout}{result.stderr}"
            )

    @pytest.mark.live_system
    def test_missing_assets_dir(self, installed_layout, clean_state):
        """Missing assets directory gives clear error."""
        backup = ASSETS_DIR.parent / "assets_backup"
        moved = False
        try:
            if ASSETS_DIR.exists():
                ASSETS_DIR.rename(backup)
                moved = True

            result = run_capsem("list", timeout=15)
            assert result.returncode != 0
            combined = (result.stdout + result.stderr).lower()
            assert "assets" in combined or "error" in combined
        finally:
            if moved and backup.exists():
                backup.rename(ASSETS_DIR)

    def test_corrupt_setup_state_does_not_trigger_setup(self, installed_layout, clean_state):
        """Corrupt setup-state.json does not trigger hidden setup work."""
        CAPSEM_DIR.mkdir(parents=True, exist_ok=True)
        state_file = CAPSEM_DIR / "setup-state.json"
        state_file.write_text("{{{invalid json")

        result = run_capsem("setup", "--non-interactive", timeout=15)
        assert result.returncode != 0
        combined = (result.stdout + result.stderr).lower()
        assert "unrecognized" in combined or "invalid" in combined

    def test_wrong_permissions_on_capsem_dir(self, installed_layout, clean_state):
        """Read-only ~/.capsem does not resurrect setup writes."""
        CAPSEM_DIR.mkdir(parents=True, exist_ok=True)
        original_mode = CAPSEM_DIR.stat().st_mode
        try:
            CAPSEM_DIR.chmod(stat.S_IRUSR | stat.S_IXUSR)  # read-only

            result = run_capsem("setup", "--non-interactive", timeout=15)
            combined = (result.stdout + result.stderr).lower()
            assert result.returncode != 0
            assert "unrecognized" in combined or "invalid" in combined
        finally:
            CAPSEM_DIR.chmod(original_mode)

    def test_stale_socket(self, installed_layout, clean_state):
        """Stale socket file doesn't prevent auto-launch."""
        RUN_DIR.mkdir(parents=True, exist_ok=True)
        stale = RUN_DIR / "service.sock"
        # Create a regular file pretending to be a socket
        stale.write_text("")

        result = run_capsem("list", timeout=15)
        # Should either connect (auto-launch cleans up) or give clear error
        combined = (result.stdout + result.stderr).lower()
        assert result.returncode == 0 or "error" in combined or "failed" in combined

        # Clean up
        stale.unlink(missing_ok=True)

    def test_version_works_without_service(self, installed_layout, clean_state):
        """capsem version works even when service is down."""
        result = run_capsem("version", timeout=5)
        assert result.returncode == 0
        assert "capsem" in result.stdout
        assert "build" in result.stdout

    @pytest.mark.live_system
    def test_service_status_works_without_install(self, installed_layout, clean_state):
        """capsem status works even when not installed."""
        result = run_capsem("status", timeout=10)
        assert result.returncode == 0
        assert "Installed:" in result.stdout

    def test_completions_work_without_service(self, installed_layout):
        """capsem completions works without service running."""
        result = run_capsem("completions", "bash", timeout=5)
        assert result.returncode == 0
        assert "capsem" in result.stdout
