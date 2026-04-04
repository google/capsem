"""Asset integrity: hashes match manifest, corrupted assets detected."""

import json
import os
import subprocess

import pytest

from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent.parent
ASSETS_DIR = PROJECT_ROOT / "assets"

pytestmark = pytest.mark.security


def _host_arch():
    return "arm64" if os.uname().machine == "arm64" else "x86_64"


def test_manifest_hash_matches_kernel():
    """b3sum of vmlinuz matches hash in manifest.json."""
    arch = _host_arch()
    kernel = ASSETS_DIR / arch / "vmlinuz"
    manifest = ASSETS_DIR / "manifest.json"

    if not kernel.exists() or not manifest.exists():
        pytest.skip("Missing kernel or manifest")

    result = subprocess.run(["b3sum", "--no-names", str(kernel)], capture_output=True, text=True)
    if result.returncode != 0:
        pytest.skip("b3sum not available")
    actual_hash = result.stdout.strip()

    data = json.loads(manifest.read_text())
    version = data["latest"]
    assets = data["releases"][version].get(arch, {}).get("assets", [])
    manifest_hash = next((a["hash"] for a in assets if a["filename"] == "vmlinuz"), None)

    assert manifest_hash is not None, "vmlinuz hash not found in manifest"
    assert actual_hash == manifest_hash, (
        f"Hash mismatch: actual={actual_hash}, manifest={manifest_hash}"
    )


def test_manifest_hash_matches_rootfs():
    """b3sum of rootfs.squashfs matches hash in manifest.json."""
    arch = _host_arch()
    rootfs = ASSETS_DIR / arch / "rootfs.squashfs"
    manifest = ASSETS_DIR / "manifest.json"

    if not rootfs.exists() or not manifest.exists():
        pytest.skip("Missing rootfs or manifest")

    result = subprocess.run(["b3sum", "--no-names", str(rootfs)], capture_output=True, text=True)
    if result.returncode != 0:
        pytest.skip("b3sum not available")
    actual_hash = result.stdout.strip()

    data = json.loads(manifest.read_text())
    version = data["latest"]
    assets = data["releases"][version].get(arch, {}).get("assets", [])
    manifest_hash = next((a["hash"] for a in assets if a["filename"] == "rootfs.squashfs"), None)

    assert manifest_hash is not None, "rootfs hash not found in manifest"
    assert actual_hash == manifest_hash


def test_manifest_hash_matches_initrd():
    """b3sum of initrd.img matches hash in manifest.json."""
    arch = _host_arch()
    initrd = ASSETS_DIR / arch / "initrd.img"
    manifest = ASSETS_DIR / "manifest.json"

    if not initrd.exists() or not manifest.exists():
        pytest.skip("Missing initrd or manifest")

    result = subprocess.run(["b3sum", "--no-names", str(initrd)], capture_output=True, text=True)
    if result.returncode != 0:
        pytest.skip("b3sum not available")
    actual_hash = result.stdout.strip()

    data = json.loads(manifest.read_text())
    version = data["latest"]
    assets = data["releases"][version].get(arch, {}).get("assets", [])
    manifest_hash = next((a["hash"] for a in assets if a["filename"] == "initrd.img"), None)

    assert manifest_hash is not None, "initrd hash not found in manifest"
    assert actual_hash == manifest_hash
