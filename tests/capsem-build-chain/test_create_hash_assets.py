"""create_hash_assets.py must clean up stale hash-tagged aliases.

When the manifest drops a release or its assets change, the prior hash-tagged
hardlinks (`<stem>-<hex16>.<ext>`) become lies: the name encodes a hash that
no longer matches the content. The script must delete those before creating
new ones. Until the Apr 2026 fix, the script only unlinked+relinked names it
already planned to create -- leaving stale names untouched and, through
subsequent builds, re-pointing them to unrelated inodes.
"""

import json
import subprocess

import pytest

from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent.parent
SCRIPT = PROJECT_ROOT / "scripts" / "create_hash_assets.py"

pytestmark = pytest.mark.build_chain


def _run(assets_dir: Path) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["python3", str(SCRIPT), str(assets_dir)],
        capture_output=True, text=True, check=True,
    )


def _arch_hashed_files(arch_dir: Path) -> set[str]:
    """Filenames in arch_dir matching the hash-tagged pattern."""
    import re
    pat = re.compile(r"^[A-Za-z0-9_]+-[0-9a-f]{16}(\.[A-Za-z0-9_.]+)?$")
    return {f.name for f in arch_dir.iterdir() if f.is_file() and pat.match(f.name)}


def _write_manifest(assets_dir: Path, initrd_hash: str) -> None:
    manifest = {
        "format": 2,
        "assets": {
            "current": "2026.0101.1",
            "releases": {
                "2026.0101.1": {
                    "date": "2026-01-01",
                    "deprecated": False,
                    "min_binary": "1.0.0",
                    "arches": {
                        "arm64": {
                            "initrd.img": {"hash": initrd_hash, "size": 100},
                        },
                    },
                },
            },
        },
        "binaries": {
            "current": "1.0.1",
            "releases": {
                "1.0.1": {
                    "date": "2026-01-01",
                    "deprecated": False,
                    "min_assets": "2026.0101.1",
                },
            },
        },
    }
    (assets_dir / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


def test_creates_expected_hash_tagged_alias(tmp_path):
    """Baseline: script creates one hash-tagged hardlink per manifest entry."""
    arch_dir = tmp_path / "arm64"
    arch_dir.mkdir()
    (arch_dir / "initrd.img").write_bytes(b"new-content")
    initrd_hash = "0000000000000000" + "f" * 48
    _write_manifest(tmp_path, initrd_hash)

    _run(tmp_path)

    expected = f"initrd-{initrd_hash[:16]}.img"
    assert expected in _arch_hashed_files(arch_dir)
    # Hardlink, same inode as the canonical file.
    assert (arch_dir / expected).stat().st_ino == (arch_dir / "initrd.img").stat().st_ino


def test_removes_stale_hash_tagged_aliases(tmp_path):
    """Hash-tagged files whose hex doesn't match any manifest hash get removed."""
    arch_dir = tmp_path / "arm64"
    arch_dir.mkdir()
    (arch_dir / "initrd.img").write_bytes(b"current-content")
    current_hash = "aaaaaaaaaaaaaaaa" + "0" * 48
    _write_manifest(tmp_path, current_hash)

    # Seed stale hash-tagged aliases from earlier builds.
    stale_a = arch_dir / "initrd-1111111111111111.img"
    stale_b = arch_dir / "initrd-2222222222222222.img"
    stale_a.write_bytes(b"old-data-a")
    stale_b.write_bytes(b"old-data-b")

    _run(tmp_path)

    remaining = _arch_hashed_files(arch_dir)
    expected = f"initrd-{current_hash[:16]}.img"
    assert remaining == {expected}, (
        f"expected only {expected}, got {remaining}"
    )
    assert not stale_a.exists()
    assert not stale_b.exists()


def test_preserves_non_hash_tagged_files(tmp_path):
    """Files that don't match the hash-tag pattern are left alone."""
    arch_dir = tmp_path / "arm64"
    arch_dir.mkdir()
    (arch_dir / "initrd.img").write_bytes(b"x")
    (arch_dir / "README").write_text("notes")
    (arch_dir / "config.toml").write_text("k=v")
    initrd_hash = "1234567890abcdef" + "0" * 48
    _write_manifest(tmp_path, initrd_hash)

    _run(tmp_path)

    assert (arch_dir / "README").exists()
    assert (arch_dir / "config.toml").exists()
    assert (arch_dir / "initrd.img").exists()
