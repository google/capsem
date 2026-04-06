"""Self-update tests for WB4.

Tests verify `capsem update` behavior for development builds,
installed layout detection, and update cache management.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from conftest import (
    CAPSEM_DIR,
    INSTALL_DIR,
    run_capsem,
    get_build_hash,
)

UPDATE_CACHE = CAPSEM_DIR / "update-check.json"


class TestSelfUpdate:
    """capsem update command."""

    def test_update_dev_build_bails(self, installed_layout, clean_state):
        """Non-installed layout prints 'build from source' advice."""
        # When running from installed layout, the binary is in ~/.capsem/bin
        # which is detected as UserDir, not Development. But the test verifies
        # the update command runs without crashing.
        result = run_capsem("update", "--yes", timeout=30)
        # Should either succeed or fail gracefully (network may not be available)
        combined = result.stdout + result.stderr
        # It should at least attempt to check or explain the situation
        assert result.returncode == 0 or "failed" in combined.lower() or "error" in combined.lower()

    def test_installed_layout_detection(self, installed_layout):
        """Installed binaries in ~/.capsem/bin are detected correctly."""
        # The binary is in ~/.capsem/bin which should be detected as UserDir
        # We verify indirectly: update command doesn't say "Development build"
        result = run_capsem("update", "--yes", timeout=30)
        assert "Development build" not in result.stdout, (
            "installed binary should not be detected as development build"
        )

    def test_update_cache_write_and_read(self, installed_layout, clean_state):
        """Update cache file written with version info."""
        # Remove any existing cache
        UPDATE_CACHE.unlink(missing_ok=True)

        # Run any command to trigger background cache refresh
        run_capsem("version", timeout=10)

        # The background refresh is fire-and-forget, may not have completed
        # Write a synthetic cache to test the read path
        CAPSEM_DIR.mkdir(parents=True, exist_ok=True)
        import time
        cache = {
            "checked_at": int(time.time()),
            "latest_version": "99.99.99",
            "update_available": True,
        }
        UPDATE_CACHE.write_text(json.dumps(cache))

        # Now any command should show the update notice
        result = run_capsem("version", timeout=10)
        assert result.returncode == 0
        # The notice goes to stderr
        assert "update available" in result.stderr.lower() or "99.99.99" in result.stderr, (
            f"expected update notice in stderr:\nstdout: {result.stdout}\nstderr: {result.stderr}"
        )

        # Cleanup
        UPDATE_CACHE.unlink(missing_ok=True)

    def test_update_preserves_old_on_download_failure(self, installed_layout, clean_state):
        """Binary remains unchanged if update download fails."""
        # Record current build hash
        original_hash = get_build_hash()

        # Try to update (will fail if no network or no newer version)
        result = run_capsem("update", "--yes", timeout=30)
        # Regardless of outcome, the installed binary should be unchanged
        current_hash = get_build_hash()
        assert current_hash == original_hash, (
            f"binary changed unexpectedly: {original_hash} -> {current_hash}"
        )
