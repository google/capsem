"""Uninstall tests for Polish: Completions + Uninstall."""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

import pytest

from .conftest import (
    CAPSEM_DIR,
    INSTALL_DIR,
    BINARIES,
    run_capsem,
    _resolve_assets_src,
    _resolve_bin_src,
)

SCRIPT = Path(__file__).resolve().parents[2] / "scripts" / "simulate-install.sh"


def _restore_runtime_from_current_build() -> None:
    subprocess.run(
        [
            "bash",
            str(SCRIPT),
            str(_resolve_bin_src()),
            str(_resolve_assets_src()),
        ],
        check=True,
        timeout=60,
    )


class TestUninstall:
    """capsem uninstall removes runtime state and preserves durable state."""

    def test_runtime_uninstall_preserves_durable_state(self, installed_layout, clean_state):
        """Uninstall with --yes removes binaries but keeps durable user data."""
        # Verify install exists first
        assert INSTALL_DIR.exists()
        for name in BINARIES:
            assert (INSTALL_DIR / name).exists()

        durable = CAPSEM_DIR / "service.toml"
        durable.write_text("# uninstall preservation sentinel\n")
        persistent = CAPSEM_DIR / "run" / "persistent" / "saved-vm"
        persistent.mkdir(parents=True, exist_ok=True)
        (persistent / "state.vz").write_text("saved")

        try:
            result = run_capsem("uninstall", "--yes", timeout=15)
            assert result.returncode == 0, (
                f"uninstall failed:\nstdout: {result.stdout}\nstderr: {result.stderr}"
            )

            assert CAPSEM_DIR.exists(), "~/.capsem should remain after runtime uninstall"
            assert not INSTALL_DIR.exists(), "~/.capsem/bin should be removed after uninstall"
            assert durable.exists(), "service settings should be preserved"
            assert persistent.exists(), "persistent VM state should be preserved"
        finally:
            if not (INSTALL_DIR / "capsem").exists():
                _restore_runtime_from_current_build()

    def test_uninstall_when_nothing_installed(self, clean_state):
        """Uninstall with no ~/.capsem gives clean message."""
        try:
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

            # We need the binary to exist somewhere to run it. Even when this
            # path skips, restore ~/.capsem/bin so later install tests do not
            # inherit the intentionally deleted product tree.
            if not Path("/usr/local/bin/capsem").exists():
                pytest.skip("capsem binary is in ~/.capsem/bin which was removed")

            result = run_capsem("uninstall", "--yes", timeout=10)
            assert result.returncode == 0
            assert "nothing to uninstall" in result.stdout.lower()
        finally:
            if not (INSTALL_DIR / "capsem").exists():
                _restore_runtime_from_current_build()

    def test_product_purge_removes_durable_state(self, installed_layout, clean_state):
        """Whole-product purge removes runtime and durable user state."""
        (CAPSEM_DIR / "service.toml").write_text("# purge sentinel\n")
        (CAPSEM_DIR / "assets" / "arm64").mkdir(parents=True, exist_ok=True)
        (CAPSEM_DIR / "assets" / "arm64" / "old-rootfs.squashfs").write_text("asset")
        persistent = CAPSEM_DIR / "run" / "persistent" / "saved-vm"
        persistent.mkdir(parents=True, exist_ok=True)
        (persistent / "state.vz").write_text("saved")

        try:
            result = run_capsem("purge", "--product", "--yes", timeout=20)
            assert result.returncode == 0, (
                f"product purge failed:\nstdout: {result.stdout}\nstderr: {result.stderr}"
            )

            assert not CAPSEM_DIR.exists(), "product purge should remove all ~/.capsem state"
        finally:
            if not (INSTALL_DIR / "capsem").exists():
                _restore_runtime_from_current_build()
