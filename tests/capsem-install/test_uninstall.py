"""Uninstall tests for Polish: Completions + Uninstall."""

from __future__ import annotations

import os
from pathlib import Path

import pytest

from conftest import (
    CAPSEM_DIR,
    INSTALL_DIR,
    BINARIES,
    run_capsem,
)


class TestUninstall:
    """capsem uninstall removes everything."""

    @pytest.mark.live_system
    def test_full_uninstall(self, installed_layout, clean_state):
        """Uninstall with --yes removes binaries and data."""
        # Verify install exists first
        assert INSTALL_DIR.exists()
        for name in BINARIES:
            assert (INSTALL_DIR / name).exists()

        result = run_capsem("uninstall", "--yes", timeout=15)
        assert result.returncode == 0, (
            f"uninstall failed:\nstdout: {result.stdout}\nstderr: {result.stderr}"
        )

        # ~/.capsem should be gone
        assert not CAPSEM_DIR.exists(), "~/.capsem should be removed after uninstall"

    def test_uninstall_when_nothing_installed(self, clean_state):
        """Uninstall with no ~/.capsem gives clean message."""
        # Remove capsem dir entirely
        import shutil
        if CAPSEM_DIR.exists():
            shutil.rmtree(CAPSEM_DIR)

        # We need the binary to exist somewhere to run it
        # This test may need to be skipped if binary is in ~/.capsem/bin
        if not Path("/usr/local/bin/capsem").exists():
            pytest.skip("capsem binary is in ~/.capsem/bin which was removed")

        result = run_capsem("uninstall", "--yes", timeout=10)
        assert result.returncode == 0
        assert "nothing to uninstall" in result.stdout.lower()
