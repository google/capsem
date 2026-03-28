"""Tests for gen_manifest.py manifest format compatibility with build.rs.

Verifies that the manifest format produced by gen_manifest.py is consumable
by the Rust build.rs hash extraction logic:
  - Per-arch B3SUMS entries produce per-arch nested format with bare filenames
  - Flat B3SUMS entries produce flat format with bare filenames
"""

import json
import subprocess
import sys
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parent.parent
GEN_MANIFEST = PROJECT_ROOT / "scripts" / "gen_manifest.py"


def _make_cargo_toml(path: Path, version: str = "0.13.0") -> Path:
    cargo = path / "Cargo.toml"
    cargo.write_text(f'[workspace.package]\nversion = "{version}"\n')
    return cargo


class TestGenManifestFormat:
    def test_per_arch_b3sums_produce_nested_format(self, tmp_path):
        """Arch-prefixed B3SUMS produce per-arch manifest with bare filenames."""
        arm64 = tmp_path / "arm64"
        arm64.mkdir()
        (arm64 / "vmlinuz").write_bytes(b"kernel")
        (arm64 / "initrd.img").write_bytes(b"initrd")
        (arm64 / "rootfs.squashfs").write_bytes(b"rootfs")

        (tmp_path / "B3SUMS").write_text(
            "aaa111  arm64/vmlinuz\n"
            "bbb222  arm64/initrd.img\n"
            "ccc333  arm64/rootfs.squashfs\n"
        )
        cargo = _make_cargo_toml(tmp_path)

        result = subprocess.run(
            [sys.executable, str(GEN_MANIFEST), str(tmp_path), str(cargo)],
            capture_output=True, text=True,
        )
        assert result.returncode == 0, result.stderr

        manifest = json.loads((tmp_path / "manifest.json").read_text())
        release = manifest["releases"]["0.13.0"]

        # Must have per-arch key
        assert "arm64" in release, f"per-arch key missing, got: {list(release.keys())}"
        assets = release["arm64"]["assets"]

        # Filenames must be bare (no arch prefix)
        filenames = {a["filename"] for a in assets}
        assert filenames == {"vmlinuz", "initrd.img", "rootfs.squashfs"}
        for asset in assets:
            assert "/" not in asset["filename"], (
                f"filename must be bare for build.rs, got: {asset['filename']}"
            )

    def test_flat_b3sums_produce_flat_format(self, tmp_path):
        """Non-prefixed B3SUMS produce flat format."""
        (tmp_path / "vmlinuz").write_bytes(b"kernel")
        (tmp_path / "initrd.img").write_bytes(b"initrd")

        (tmp_path / "B3SUMS").write_text(
            "aaa111  vmlinuz\n"
            "bbb222  initrd.img\n"
        )
        cargo = _make_cargo_toml(tmp_path)

        result = subprocess.run(
            [sys.executable, str(GEN_MANIFEST), str(tmp_path), str(cargo)],
            capture_output=True, text=True,
        )
        assert result.returncode == 0, result.stderr

        manifest = json.loads((tmp_path / "manifest.json").read_text())
        release = manifest["releases"]["0.13.0"]

        assert "assets" in release
        filenames = {a["filename"] for a in release["assets"]}
        assert filenames == {"vmlinuz", "initrd.img"}

    def test_multi_arch_b3sums(self, tmp_path):
        """Multiple arch prefixes produce multiple arch keys."""
        for arch in ("arm64", "x86_64"):
            d = tmp_path / arch
            d.mkdir()
            (d / "vmlinuz").write_bytes(b"kernel")

        (tmp_path / "B3SUMS").write_text(
            "aaa111  arm64/vmlinuz\n"
            "ddd444  x86_64/vmlinuz\n"
        )
        cargo = _make_cargo_toml(tmp_path)

        result = subprocess.run(
            [sys.executable, str(GEN_MANIFEST), str(tmp_path), str(cargo)],
            capture_output=True, text=True,
        )
        assert result.returncode == 0, result.stderr

        manifest = json.loads((tmp_path / "manifest.json").read_text())
        release = manifest["releases"]["0.13.0"]
        assert "arm64" in release
        assert "x86_64" in release
        assert release["arm64"]["assets"][0]["hash"] == "aaa111"
        assert release["x86_64"]["assets"][0]["hash"] == "ddd444"
