"""Uninstall tests for Polish: Completions + Uninstall."""

from __future__ import annotations

import os
from pathlib import Path

from .conftest import (
    CAPSEM_DIR,
    INSTALL_DIR,
    BINARIES,
    run_capsem,
)


class TestUninstall:
    """capsem uninstall removes runtime state and preserves durable state."""

    def test_runtime_uninstall_preserves_durable_state(self, installed_layout, clean_state):
        """Uninstall with --yes removes binaries but keeps durable user data."""
        # Verify install exists first
        assert INSTALL_DIR.exists()
        for name in BINARIES:
            assert (INSTALL_DIR / name).exists()

        durable = CAPSEM_DIR / "user.toml"
        durable.write_text("# uninstall preservation sentinel\n")
        persistent = CAPSEM_DIR / "run" / "persistent" / "saved-vm"
        persistent.mkdir(parents=True, exist_ok=True)
        (persistent / "state.vz").write_text("saved")

        result = run_capsem("uninstall", "--yes", timeout=15)
        assert result.returncode == 0, (
            f"uninstall failed:\nstdout: {result.stdout}\nstderr: {result.stderr}"
        )

        assert CAPSEM_DIR.exists(), "~/.capsem should remain after runtime uninstall"
        assert not INSTALL_DIR.exists(), "~/.capsem/bin should be removed after uninstall"
        assert durable.exists(), "user config should be preserved"
        assert persistent.exists(), "persistent VM state should be preserved"

    def test_uninstall_when_nothing_installed(self, clean_state):
        """Uninstall with no ~/.capsem gives clean message."""
        # Remove capsem dir entirely. Overlayfs workdirs may be mode 000, so
        # walk and chmod before rmtree.
        import shutil
        import stat as _stat
        if CAPSEM_DIR.exists():
            for root, dirs, _files in os.walk(CAPSEM_DIR):
                for d in dirs:
                    p = Path(root) / d
                    try:
                        p.chmod(_stat.S_IRWXU)
                    except OSError:
                        pass
            shutil.rmtree(CAPSEM_DIR)

        # We need the binary to exist somewhere to run it
        # This test may need to be skipped if binary is in ~/.capsem/bin
        if not Path("/usr/local/bin/capsem").exists():
            import pytest

            pytest.skip("capsem binary is in ~/.capsem/bin which was removed")

        result = run_capsem("uninstall", "--yes", timeout=10)
        assert result.returncode == 0
        assert "nothing to uninstall" in result.stdout.lower()

    def test_product_purge_removes_durable_state(self, installed_layout, clean_state):
        """Whole-product purge removes runtime and durable user state."""
        (CAPSEM_DIR / "user.toml").write_text("# purge sentinel\n")
        (CAPSEM_DIR / "assets" / "arm64").mkdir(parents=True, exist_ok=True)
        (CAPSEM_DIR / "assets" / "arm64" / "old-rootfs.squashfs").write_text("asset")
        persistent = CAPSEM_DIR / "run" / "persistent" / "saved-vm"
        persistent.mkdir(parents=True, exist_ok=True)
        (persistent / "state.vz").write_text("saved")

        result = run_capsem("purge", "--product", "--yes", timeout=20)
        assert result.returncode == 0, (
            f"product purge failed:\nstdout: {result.stdout}\nstderr: {result.stderr}"
        )

        assert not CAPSEM_DIR.exists(), "product purge should remove all ~/.capsem state"
