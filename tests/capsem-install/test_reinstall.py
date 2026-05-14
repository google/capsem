"""Reinstall verification tests.

Verifies that simulate-install.sh actually replaces binaries with the new
build. Uses the build hash embedded by build.rs to distinguish builds.
"""

from __future__ import annotations

import hashlib
import os
import subprocess
import json
import filecmp
from pathlib import Path

import pytest

from .conftest import (
    INSTALL_DIR,
    BINARIES,
    get_build_hash,
    run_capsem,
    _resolve_bin_src,
)

SCRIPT = Path(__file__).parent.parent.parent / "scripts" / "simulate-install.sh"


def _simulate_install_from_current_build() -> None:
    bin_src = _resolve_bin_src()
    assets_src = os.environ.get("CAPSEM_ASSETS_SRC", "assets")
    subprocess.run(
        ["bash", str(SCRIPT), str(bin_src), assets_src],
        check=True,
        timeout=60,
    )


def _assert_status_has_no_runtime_layout_issues() -> None:
    result = run_capsem("status", "--json", timeout=10)
    assert result.stdout, f"status --json should print a report: {result.stderr}"
    report = json.loads(result.stdout)
    codes = {issue["code"] for issue in report["issues"]}
    assert "host_binary_missing" not in codes, report
    assert "host_binary_not_executable" not in codes, report
    assert "host_binary_version_mismatch" not in codes, report
    assert report["checks"]["host"]["state"] == "ok"


@pytest.fixture
def _needs_cargo():
    """Skip if cargo is not available (e.g., in some Docker images)."""
    result = subprocess.run(["cargo", "--version"], capture_output=True)
    if result.returncode != 0:
        pytest.skip("cargo not available")


class TestReinstall:
    """Verify reinstall replaces binaries."""

    @pytest.mark.live_system
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
        """All host binaries must exist and be executable after install."""
        for name in BINARIES:
            binary = INSTALL_DIR / name
            assert binary.exists(), f"missing after install: {name}"
            assert os.access(binary, os.X_OK), f"not executable: {name}"
            # Verify non-zero size (not an empty stub)
            assert binary.stat().st_size > 0, f"empty binary: {name}"

    def test_reinstall_after_runtime_uninstall_restores_status_layout(
        self, installed_layout, clean_state
    ):
        """A clean runtime uninstall followed by install restores all helpers."""
        result = run_capsem("uninstall", "--yes", timeout=15)
        assert result.returncode == 0, (
            f"uninstall failed:\nstdout: {result.stdout}\nstderr: {result.stderr}"
        )
        assert not INSTALL_DIR.exists()

        _simulate_install_from_current_build()

        for name in BINARIES:
            assert (INSTALL_DIR / name).exists(), f"missing after reinstall: {name}"
        _assert_status_has_no_runtime_layout_issues()

    def test_reinstall_over_existing_replaces_corrupt_helper(
        self, installed_layout, clean_state
    ):
        """Install over an existing tree must replace stale helper contents."""
        helper = INSTALL_DIR / "capsem-gateway"
        helper.unlink()
        helper.write_text("#!/bin/sh\nprintf 'capsem-gateway 0.0.0\\n'\n", encoding="utf-8")
        helper.chmod(0o755)

        before = run_capsem("status", "--json", timeout=10)
        before_report = json.loads(before.stdout)
        assert any(
            issue["code"] == "host_binary_version_mismatch"
            and issue["details"]["name"] == "capsem-gateway"
            for issue in before_report["issues"]
        )

        _simulate_install_from_current_build()

        assert filecmp.cmp(_resolve_bin_src() / "capsem-gateway", helper, shallow=False)
        _assert_status_has_no_runtime_layout_issues()
