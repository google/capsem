"""Verify manifest.json hashes match actual asset files."""

import json
import os
import subprocess

import pytest

from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent.parent
ASSETS_DIR = PROJECT_ROOT / "assets"


def host_arch():
    return "arm64" if os.uname().machine == "arm64" else "x86_64"

pytestmark = pytest.mark.build_chain


@pytest.fixture
def manifest():
    arch = host_arch()
    manifest_path = ASSETS_DIR / arch / "manifest.json"
    if not manifest_path.exists():
        pytest.skip(f"manifest.json not found at {manifest_path}")
    return json.loads(manifest_path.read_text()), ASSETS_DIR / arch


def test_manifest_valid_json(manifest):
    """manifest.json is valid JSON with expected structure."""
    data, _ = manifest
    assert isinstance(data, dict), "manifest should be a JSON object"
    assert len(data) > 0, "manifest is empty"


def test_manifest_files_exist(manifest):
    """Every file listed in manifest exists on disk."""
    data, assets_dir = manifest
    for filename in data:
        filepath = assets_dir / filename
        assert filepath.exists(), f"Manifest lists {filename} but file not found"


def test_manifest_hashes_match(manifest):
    """b3sum of each asset file matches the hash in manifest.json."""
    data, assets_dir = manifest
    for filename, expected_hash in data.items():
        filepath = assets_dir / filename
        if not filepath.exists():
            continue
        result = subprocess.run(
            ["b3sum", "--no-names", str(filepath)],
            capture_output=True, text=True,
        )
        if result.returncode != 0:
            pytest.skip("b3sum not installed")
        actual_hash = result.stdout.strip()
        assert actual_hash == expected_hash, (
            f"{filename}: manifest says {expected_hash}, actual is {actual_hash}"
        )


def test_no_extra_assets(manifest):
    """No unlisted asset files in the arch directory (except manifest.json itself)."""
    data, assets_dir = manifest
    manifest_files = set(data.keys()) | {"manifest.json"}
    actual_files = {f.name for f in assets_dir.iterdir() if f.is_file()}
    extra = actual_files - manifest_files
    assert not extra, f"Unlisted asset files: {extra}"
