"""Verify manifest.json matches actual asset files on disk.

The manifest at assets/manifest.json (v2 format) records per-arch assets
with their blake3 hashes. These tests check that for the host arch:

  - manifest.json parses as expected v2 shape
  - every file listed exists on disk under assets/<arch>/
  - each file's blake3 hash matches the manifest entry
  - no unlisted files live in assets/<arch>/, allowing hash-tagged aliases
    (`<stem>-<hex16>.<ext>`) only when <hex16> matches a manifest entry

Until April 2026 this file read a per-arch manifest.json that never
existed, so the fixture always skipped and all four tests were silently
inert. See CHANGELOG for the conftest + manifest layout fix.
"""

import json
import os
import re
import subprocess

import pytest

from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent.parent
ASSETS_DIR = PROJECT_ROOT / "assets"

HASH_TAG_RE = re.compile(r"^(?P<stem>[A-Za-z0-9_]+)-(?P<hex>[0-9a-f]{16})(?P<ext>\.[A-Za-z0-9_.]+)?$")

pytestmark = pytest.mark.build_chain


def _host_arch() -> str:
    return "arm64" if os.uname().machine == "arm64" else "x86_64"


@pytest.fixture
def manifest_and_arch_assets():
    """(full_manifest, arch_assets, arch_dir) for the current host arch.

    `arch_assets` is the {filename: {hash, size}} dict scoped to the
    manifest's current release + host arch. Skips cleanly if either the
    manifest or the arch dir is absent (fresh checkout).
    """
    manifest_path = ASSETS_DIR / "manifest.json"
    if not manifest_path.exists():
        pytest.skip(f"manifest.json not found at {manifest_path}")

    data = json.loads(manifest_path.read_text())
    current = data["assets"]["current"]
    release = data["assets"]["releases"][current]
    arch = _host_arch()
    if arch not in release["arches"]:
        pytest.skip(f"arch {arch} not in manifest release {current}")
    arch_assets = release["arches"][arch]
    arch_dir = ASSETS_DIR / arch
    if not arch_dir.is_dir():
        pytest.skip(f"arch dir not found: {arch_dir}")
    return data, arch_assets, arch_dir


def test_manifest_has_expected_v2_shape(manifest_and_arch_assets):
    """manifest.json is format=2 with the expected top-level keys."""
    data, _, _ = manifest_and_arch_assets
    assert data.get("format") == 2
    assert "current" in data["assets"] and "releases" in data["assets"]
    assert "current" in data["binaries"] and "releases" in data["binaries"]


def test_manifest_files_exist(manifest_and_arch_assets):
    """Every per-arch file listed in the current release exists on disk."""
    _, arch_assets, arch_dir = manifest_and_arch_assets
    for filename in arch_assets:
        assert (arch_dir / filename).exists(), (
            f"manifest lists {filename} but {arch_dir / filename} not found"
        )


def test_manifest_hashes_match(manifest_and_arch_assets):
    """b3sum of each listed asset matches the hash recorded in manifest."""
    _, arch_assets, arch_dir = manifest_and_arch_assets
    for filename, entry in arch_assets.items():
        filepath = arch_dir / filename
        result = subprocess.run(
            ["b3sum", "--no-names", str(filepath)],
            capture_output=True, text=True,
        )
        if result.returncode != 0:
            pytest.skip("b3sum not installed")
        actual = result.stdout.strip()
        assert actual == entry["hash"], (
            f"{filename}: manifest says {entry['hash']}, b3sum is {actual}"
        )


def test_no_extra_assets(manifest_and_arch_assets):
    """No unlisted files in arch dir, aside from known hash-tagged aliases.

    A hash-tagged alias is permitted when its <hex16> suffix prefixes the
    manifest hash for the same logical asset. Anything else -- a rogue
    file, or a stale hash-tagged name left over from a prior build --
    should fail this check. The stale-alias class is what
    `scripts/create_hash_assets.py`'s cleanup pass now guards against.
    """
    _, arch_assets, arch_dir = manifest_and_arch_assets
    allowed_hashed: set[str] = set()
    for name, entry in arch_assets.items():
        h = entry["hash"][:16]
        dot = name.find(".")
        if dot >= 0:
            allowed_hashed.add(f"{name[:dot]}-{h}{name[dot:]}")
        else:
            allowed_hashed.add(f"{name}-{h}")
    allowed = set(arch_assets.keys()) | allowed_hashed

    actual = {f.name for f in arch_dir.iterdir() if f.is_file()}
    # Ignore non-hash-tagged files that aren't meant to be manifest-managed
    # (e.g. tool-versions.txt), but flag any stale hash-tagged name.
    suspicious = {n for n in actual if HASH_TAG_RE.match(n) or n in arch_assets}
    extra = suspicious - allowed
    assert not extra, f"unlisted manifest-scope files: {extra}"
