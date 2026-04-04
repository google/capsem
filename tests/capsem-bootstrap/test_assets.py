"""Asset manifest, hashes, and architecture verification.

These tests do NOT boot VMs -- they validate the build artifacts.
"""

import json
import os
import subprocess

import pytest

from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent.parent
ASSETS_DIR = PROJECT_ROOT / "assets"

pytestmark = pytest.mark.bootstrap


def _host_arch():
    return "arm64" if os.uname().machine == "arm64" else "x86_64"


class TestManifest:

    def test_manifest_exists(self):
        manifest = ASSETS_DIR / "manifest.json"
        assert manifest.exists(), f"manifest.json not found at {manifest}"

    def test_manifest_valid_json(self):
        manifest = ASSETS_DIR / "manifest.json"
        if not manifest.exists():
            pytest.skip("No manifest.json")
        data = json.loads(manifest.read_text())
        assert "latest" in data
        assert "releases" in data

    def test_manifest_version_matches_cargo(self):
        manifest = ASSETS_DIR / "manifest.json"
        cargo_toml = PROJECT_ROOT / "Cargo.toml"
        if not manifest.exists():
            pytest.skip("No manifest.json")

        data = json.loads(manifest.read_text())
        cargo_text = cargo_toml.read_text()
        # Extract version from workspace Cargo.toml
        for line in cargo_text.splitlines():
            if line.strip().startswith("version") and "=" in line:
                cargo_version = line.split("=")[1].strip().strip('"')
                break
        else:
            pytest.skip("Could not find version in Cargo.toml")

        assert data["latest"] == cargo_version, (
            f"manifest latest={data['latest']} != Cargo.toml version={cargo_version}"
        )

    def test_manifest_has_host_arch(self):
        manifest = ASSETS_DIR / "manifest.json"
        if not manifest.exists():
            pytest.skip("No manifest.json")
        data = json.loads(manifest.read_text())
        arch = _host_arch()
        latest = data["latest"]
        assert arch in data["releases"].get(latest, {}), (
            f"No {arch} entry in manifest for version {latest}"
        )


class TestAssetFiles:

    def test_kernel_exists(self):
        arch = _host_arch()
        kernel = ASSETS_DIR / arch / "vmlinuz"
        assert kernel.exists(), f"Kernel not found: {kernel}"

    def test_initrd_exists(self):
        arch = _host_arch()
        initrd = ASSETS_DIR / arch / "initrd.img"
        assert initrd.exists(), f"Initrd not found: {initrd}"

    def test_rootfs_exists(self):
        arch = _host_arch()
        rootfs = ASSETS_DIR / arch / "rootfs.squashfs"
        assert rootfs.exists(), f"Rootfs not found: {rootfs}"

    def test_initrd_valid_gzip(self):
        arch = _host_arch()
        initrd = ASSETS_DIR / arch / "initrd.img"
        if not initrd.exists():
            pytest.skip("No initrd")
        result = subprocess.run(["gunzip", "-t", str(initrd)], capture_output=True)
        assert result.returncode == 0, f"initrd is not valid gzip: {result.stderr.decode()}"


class TestHashes:

    def test_b3sums_file_exists(self):
        b3sums = ASSETS_DIR / "B3SUMS"
        if not b3sums.exists():
            pytest.skip("No B3SUMS file")
        assert b3sums.stat().st_size > 0

    def test_b3sums_match_actual(self):
        b3sums = ASSETS_DIR / "B3SUMS"
        if not b3sums.exists():
            pytest.skip("No B3SUMS file")

        # Check if b3sum tool is available
        result = subprocess.run(["b3sum", "--version"], capture_output=True)
        if result.returncode != 0:
            pytest.skip("b3sum tool not installed")

        result = subprocess.run(
            ["b3sum", "--check", str(b3sums)],
            capture_output=True, text=True,
            cwd=str(ASSETS_DIR),
        )
        assert result.returncode == 0, f"Hash mismatch:\n{result.stdout}\n{result.stderr}"
