"""Asset integrity: hashes match manifest, corrupted assets detected."""

import subprocess
from pathlib import Path

import pytest
from capsem_builder.release.tools.prove_release_runtime_assets import (
    host_architecture as _host_arch,
)
from helpers.constants import ASSETS_DIR
from helpers.stopped_workspace import manifest_asset_pins

PROJECT_ROOT = Path(__file__).parent.parent.parent

pytestmark = pytest.mark.security


def _arch_manifest(arch):
    """Read the selected boot-asset hashes from local or release manifests."""
    manifest = ASSETS_DIR / "manifest.json"
    if not manifest.exists():
        pytest.skip("Missing manifest")
    return {
        pin["name"]: {"hash": pin["hash"].removeprefix("blake3:")}
        for pin in manifest_asset_pins(ASSETS_DIR, architecture=arch).values()
    }


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


def _rootfs_filename():
    arch = _host_arch()
    entries = _arch_manifest(arch)
    if "rootfs.erofs" in entries:
        return "rootfs.erofs"
    pytest.fail(f"manifest has no rootfs.erofs entry for {arch}: {sorted(entries)}")


def test_manifest_hash_matches_kernel():
    """b3sum of vmlinuz matches hash in manifest.json."""
    _check("vmlinuz")


def test_manifest_hash_matches_rootfs():
    """b3sum of the canonical rootfs matches hash in manifest.json."""
    _check(_rootfs_filename())


def test_manifest_hash_matches_initrd():
    """b3sum of initrd.img matches hash in manifest.json."""
    _check("initrd.img")
