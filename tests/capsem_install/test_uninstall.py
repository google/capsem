"""Uninstall tests for Polish: Completions + Uninstall."""

from __future__ import annotations

import contextlib
import os
from pathlib import Path

import pytest

from .conftest import (
    BINARIES,
    CAPSEM_DIR,
    INSTALL_DIR,
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
        if os.environ.get("CAPSEM_DEB_INSTALLED") == "1":
            pytest.skip("removes the package harness install; covered by live-system uninstall tests")

        # Remove capsem dir entirely. Overlayfs workdirs may be mode 000, so
        # walk and chmod before rmtree.
        import shutil
        import stat as _stat
        if CAPSEM_DIR.exists():
            for root, dirs, _files in os.walk(CAPSEM_DIR):
                for d in dirs:
                    p = Path(root) / d
                    with contextlib.suppress(OSError):
                        p.chmod(_stat.S_IRWXU)
            shutil.rmtree(CAPSEM_DIR)

        # We need the binary to exist somewhere to run it
        # This test may need to be skipped if binary is in ~/.capsem/bin
        if not Path("/usr/local/bin/capsem").exists():
            pytest.skip("capsem binary is in ~/.capsem/bin which was removed")

        result = run_capsem("uninstall", "--yes", timeout=10)
        assert result.returncode == 0
        assert "nothing to uninstall" in result.stdout.lower()
