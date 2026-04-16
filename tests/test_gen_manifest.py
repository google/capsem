"""Tests for gen_manifest.py v2 manifest format.

Verifies that the manifest format produced by gen_manifest.py is a valid v2
manifest with separate assets/binaries sections, per-arch asset maps, and
correct hash/size entries.
"""

import json
import subprocess
import sys
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parent.parent
GEN_MANIFEST = PROJECT_ROOT / "scripts" / "gen_manifest.py"


def _make_cargo_toml(path: Path, version: str = "1.0.1000000000") -> Path:
    cargo = path / "Cargo.toml"
    cargo.write_text(f'[workspace.package]\nversion = "{version}"\n')
    return cargo


class TestGenManifestV2:
    def test_per_arch_b3sums_produce_v2_format(self, tmp_path):
        """Arch-prefixed B3SUMS produce v2 manifest with per-arch asset maps."""
        arm64 = tmp_path / "arm64"
        arm64.mkdir()
        (arm64 / "vmlinuz").write_bytes(b"kernel")
        (arm64 / "initrd.img").write_bytes(b"initrd")
        (arm64 / "rootfs.squashfs").write_bytes(b"rootfs")

        (tmp_path / "B3SUMS").write_text(
            "aaa111aaa111aaa111aaa111aaa111aaa111aaa111aaa111aaa111aaa111aaa1  arm64/vmlinuz\n"
            "bbb222bbb222bbb222bbb222bbb222bbb222bbb222bbb222bbb222bbb222bbb2  arm64/initrd.img\n"
            "ccc333ccc333ccc333ccc333ccc333ccc333ccc333ccc333ccc333ccc333ccc3  arm64/rootfs.squashfs\n"
        )
        cargo = _make_cargo_toml(tmp_path)

        result = subprocess.run(
            [sys.executable, str(GEN_MANIFEST), str(tmp_path), str(cargo)],
            capture_output=True, text=True,
        )
        assert result.returncode == 0, result.stderr

        manifest = json.loads((tmp_path / "manifest.json").read_text())

        # v2 format marker
        assert manifest["format"] == 2

        # Assets section
        assert "assets" in manifest
        asset_ver = manifest["assets"]["current"]
        release = manifest["assets"]["releases"][asset_ver]
        assert "arches" in release
        assert "arm64" in release["arches"]

        arm64_assets = release["arches"]["arm64"]
        assert set(arm64_assets.keys()) == {"vmlinuz", "initrd.img", "rootfs.squashfs"}
        assert arm64_assets["vmlinuz"]["hash"].startswith("aaa111")
        assert arm64_assets["vmlinuz"]["size"] == 6  # len(b"kernel")

        # Binaries section
        assert "binaries" in manifest
        assert manifest["binaries"]["current"] == "1.0.1000000000"
        bin_rel = manifest["binaries"]["releases"]["1.0.1000000000"]
        assert bin_rel["min_assets"] == asset_ver

        # Metadata
        assert release["deprecated"] is False
        assert "date" in release

    def test_flat_b3sums_use_unknown_arch(self, tmp_path):
        """Non-prefixed B3SUMS entries get arch 'unknown'."""
        (tmp_path / "vmlinuz").write_bytes(b"kernel")

        (tmp_path / "B3SUMS").write_text(
            "aaa111aaa111aaa111aaa111aaa111aaa111aaa111aaa111aaa111aaa111aaa1  vmlinuz\n"
        )
        cargo = _make_cargo_toml(tmp_path)

        result = subprocess.run(
            [sys.executable, str(GEN_MANIFEST), str(tmp_path), str(cargo)],
            capture_output=True, text=True,
        )
        assert result.returncode == 0, result.stderr

        manifest = json.loads((tmp_path / "manifest.json").read_text())
        asset_ver = manifest["assets"]["current"]
        release = manifest["assets"]["releases"][asset_ver]
        assert "unknown" in release["arches"]
        assert "vmlinuz" in release["arches"]["unknown"]

    def test_multi_arch_b3sums(self, tmp_path):
        """Multiple arch prefixes produce multiple arch keys."""
        for arch in ("arm64", "x86_64"):
            d = tmp_path / arch
            d.mkdir()
            (d / "vmlinuz").write_bytes(b"kernel")

        (tmp_path / "B3SUMS").write_text(
            "aaa111aaa111aaa111aaa111aaa111aaa111aaa111aaa111aaa111aaa111aaa1  arm64/vmlinuz\n"
            "ddd444ddd444ddd444ddd444ddd444ddd444ddd444ddd444ddd444ddd444ddd4  x86_64/vmlinuz\n"
        )
        cargo = _make_cargo_toml(tmp_path)

        result = subprocess.run(
            [sys.executable, str(GEN_MANIFEST), str(tmp_path), str(cargo)],
            capture_output=True, text=True,
        )
        assert result.returncode == 0, result.stderr

        manifest = json.loads((tmp_path / "manifest.json").read_text())
        asset_ver = manifest["assets"]["current"]
        release = manifest["assets"]["releases"][asset_ver]
        assert "arm64" in release["arches"]
        assert "x86_64" in release["arches"]
        assert release["arches"]["arm64"]["vmlinuz"]["hash"].startswith("aaa111")
        assert release["arches"]["x86_64"]["vmlinuz"]["hash"].startswith("ddd444")

    def test_patch_auto_increment(self, tmp_path):
        """Running gen_manifest twice on the same day increments the patch."""
        (tmp_path / "vmlinuz").write_bytes(b"kernel")
        (tmp_path / "B3SUMS").write_text(
            "aaa111aaa111aaa111aaa111aaa111aaa111aaa111aaa111aaa111aaa111aaa1  vmlinuz\n"
        )
        cargo = _make_cargo_toml(tmp_path)

        # First run
        subprocess.run(
            [sys.executable, str(GEN_MANIFEST), str(tmp_path), str(cargo)],
            capture_output=True, text=True, check=True,
        )
        m1 = json.loads((tmp_path / "manifest.json").read_text())
        v1 = m1["assets"]["current"]
        assert v1.endswith(".1")

        # Second run
        subprocess.run(
            [sys.executable, str(GEN_MANIFEST), str(tmp_path), str(cargo)],
            capture_output=True, text=True, check=True,
        )
        m2 = json.loads((tmp_path / "manifest.json").read_text())
        v2 = m2["assets"]["current"]
        assert v2.endswith(".2")
