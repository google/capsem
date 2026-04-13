"""Reinstall verification tests.

Verifies that simulate-install.sh actually replaces binaries with the new
build. Uses the build hash embedded by build.rs to distinguish builds.
"""

from __future__ import annotations

import hashlib
import os
import subprocess
from pathlib import Path

import pytest

from conftest import (
    INSTALL_DIR,
    BINARIES,
    get_build_hash,
)

SCRIPT = Path(__file__).parent.parent.parent / "scripts" / "simulate-install.sh"


@pytest.fixture
def _needs_cargo():
    """Skip if cargo is not available (e.g., in some Docker images)."""
    result = subprocess.run(["cargo", "--version"], capture_output=True)
    if result.returncode != 0:
        pytest.skip("cargo not available")


class TestReinstall:
    """Verify reinstall replaces binaries."""

    def test_reinstall_replaces_binaries(self, clean_state, _needs_cargo):
        """Compile v1, install, recompile v2, install -- verify v2."""
        capsem_bin = INSTALL_DIR / "capsem"
        bin_src = os.environ.get("CAPSEM_BIN_SRC", "target/debug")
        assets_src = os.environ.get("CAPSEM_ASSETS_SRC", "assets")

        # Build 1: compile and install
        subprocess.run(["cargo", "build", "-p", "capsem"], check=True, timeout=300)
        subprocess.run(
            ["bash", str(SCRIPT), bin_src, assets_src],
            check=True, timeout=60,
        )
        hash_1 = get_build_hash()
        file_hash_1 = hashlib.sha256(capsem_bin.read_bytes()).hexdigest()

        # Force recompile by cleaning the capsem crate
        subprocess.run(["cargo", "clean", "-p", "capsem"], check=True, timeout=60)

        # Build 2: compile and install
        subprocess.run(["cargo", "build", "-p", "capsem"], check=True, timeout=300)
        subprocess.run(
            ["bash", str(SCRIPT), bin_src, assets_src],
            check=True, timeout=60,
        )
        hash_2 = get_build_hash()
        file_hash_2 = hashlib.sha256(capsem_bin.read_bytes()).hexdigest()

        # The installed binary must be the NEW build
        assert hash_1 != hash_2, "Build hashes should differ after recompile"
        assert file_hash_1 != file_hash_2, "File hashes should differ after reinstall"

    def test_all_binaries_updated(self, installed_layout):
        """All 6 binaries must exist and be executable after install."""
        for name in BINARIES:
            binary = INSTALL_DIR / name
            assert binary.exists(), f"missing after install: {name}"
            assert os.access(binary, os.X_OK), f"not executable: {name}"
            # Verify non-zero size (not an empty stub)
            assert binary.stat().st_size > 0, f"empty binary: {name}"
