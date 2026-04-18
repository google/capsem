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


def _arch_manifest(arch):
    """Return the per-arch asset entries from manifest.json (v2) or skip."""
    manifest = ASSETS_DIR / "manifest.json"
    if not manifest.exists():
        pytest.skip("Missing manifest")
    data = json.loads(manifest.read_text())
    current = data.get("assets", {}).get("current")
    if current is None:
        pytest.skip("manifest missing assets.current")
    arches = data["assets"]["releases"].get(current, {}).get("arches", {})
    entries = arches.get(arch)
    if entries is None:
        pytest.skip(f"manifest has no {arch} entry for version {current}")
    return entries


def _b3sum(path):
    result = subprocess.run(["b3sum", "--no-names", str(path)], capture_output=True, text=True)
    if result.returncode != 0:
        pytest.skip("b3sum not available")
    return result.stdout.strip()


def _check(filename):
    arch = _host_arch()
    asset = ASSETS_DIR / arch / filename
    if not asset.exists():
        pytest.skip(f"Missing asset {asset}")

    actual_hash = _b3sum(asset)
    entries = _arch_manifest(arch)
    entry = entries.get(filename)
    assert entry is not None, f"{filename} hash not found in manifest for {arch}"
    manifest_hash = entry["hash"]
    assert actual_hash == manifest_hash, (
        f"Hash mismatch for {filename}: actual={actual_hash}, manifest={manifest_hash}"
    )


def test_manifest_hash_matches_kernel():
    """b3sum of vmlinuz matches hash in manifest.json."""
    _check("vmlinuz")


def test_manifest_hash_matches_rootfs():
    """b3sum of rootfs.squashfs matches hash in manifest.json."""
    _check("rootfs.squashfs")


def test_manifest_hash_matches_initrd():
    """b3sum of initrd.img matches hash in manifest.json."""
    _check("initrd.img")
