"""Tests for capsem-admin manifest generation.

Verifies that the public `capsem-admin manifest generate <assets_dir>` rail
produces a valid v2 manifest with separate assets/binaries sections,
per-arch asset maps, and correct hash/size entries.
"""

import json
import subprocess
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parent.parent


def _write_asset_set(base: Path, arch: str | None = None, marker: bytes = b"") -> None:
    output = base / arch if arch else base
    output.mkdir(parents=True, exist_ok=True)
    (output / "vmlinuz").write_bytes(b"kernel" + marker)
    (output / "initrd.img").write_bytes(b"initrd" + marker)
    (output / "rootfs.erofs").write_bytes(b"rootfs" + marker)


def _run_admin_manifest_generate(path: Path, version: str = "1.0.1000000000") -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [
            "cargo",
            "run",
            "-p",
            "capsem-admin",
            "--",
            "manifest",
            "generate",
            str(path),
            "--version",
            version,
        ],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
    )


class TestGenManifestV2:
    def test_per_arch_b3sums_produce_v2_format(self, tmp_path):
        """Arch-prefixed B3SUMS produce v2 manifest with per-arch asset maps."""
        _write_asset_set(tmp_path, "arm64")

        result = _run_admin_manifest_generate(tmp_path)
        assert result.returncode == 0, result.stderr

        manifest = json.loads((tmp_path / "manifest.json").read_text())
        b3sums = (tmp_path / "B3SUMS").read_text()
        assert "arm64/vmlinuz" in b3sums
        assert "arm64/initrd.img" in b3sums
        assert "arm64/rootfs.erofs" in b3sums

        # v2 format marker
        assert manifest["format"] == 2
        assert manifest["refresh_policy"] == "24h"

        # Assets section
        assert "assets" in manifest
        asset_ver = manifest["assets"]["current"]
        release = manifest["assets"]["releases"][asset_ver]
        assert "arches" in release
        assert "arm64" in release["arches"]

        arm64_assets = release["arches"]["arm64"]
        assert set(arm64_assets.keys()) == {"vmlinuz", "initrd.img", "rootfs.erofs"}
        assert len(arm64_assets["vmlinuz"]["hash"]) == 64
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
        _write_asset_set(tmp_path)

        result = _run_admin_manifest_generate(tmp_path)
        assert result.returncode == 0, result.stderr

        manifest = json.loads((tmp_path / "manifest.json").read_text())
        asset_ver = manifest["assets"]["current"]
        release = manifest["assets"]["releases"][asset_ver]
        assert "unknown" in release["arches"]
        assert set(release["arches"]["unknown"]) == {"vmlinuz", "initrd.img", "rootfs.erofs"}

    def test_multi_arch_b3sums(self, tmp_path):
        """Multiple arch prefixes produce multiple arch keys."""
        for arch in ("arm64", "x86_64"):
            _write_asset_set(tmp_path, arch, marker=arch.encode())

        result = _run_admin_manifest_generate(tmp_path)
        assert result.returncode == 0, result.stderr

        manifest = json.loads((tmp_path / "manifest.json").read_text())
        asset_ver = manifest["assets"]["current"]
        release = manifest["assets"]["releases"][asset_ver]
        assert "arm64" in release["arches"]
        assert "x86_64" in release["arches"]
        assert release["arches"]["arm64"]["vmlinuz"]["hash"] != release["arches"]["x86_64"]["vmlinuz"]["hash"]

    def test_identical_assets_reuse_current_release(self, tmp_path):
        """Running admin manifest generation twice for identical assets does not mint a release."""
        _write_asset_set(tmp_path)

        # First run
        _run_admin_manifest_generate(tmp_path).check_returncode()
        m1 = json.loads((tmp_path / "manifest.json").read_text())
        v1 = m1["assets"]["current"]
        assert v1.endswith(".1")

        # Second run
        _run_admin_manifest_generate(tmp_path).check_returncode()
        m2 = json.loads((tmp_path / "manifest.json").read_text())
        v2 = m2["assets"]["current"]
        assert v2 == v1

    def test_changed_assets_increment_release(self, tmp_path):
        """A changed asset map gets a new asset release."""
        _write_asset_set(tmp_path)

        _run_admin_manifest_generate(tmp_path).check_returncode()
        m1 = json.loads((tmp_path / "manifest.json").read_text())
        v1 = m1["assets"]["current"]

        (tmp_path / "vmlinuz").write_bytes(b"kernel-changed")
        _run_admin_manifest_generate(tmp_path).check_returncode()
        m2 = json.loads((tmp_path / "manifest.json").read_text())
        v2 = m2["assets"]["current"]

        assert v1.endswith(".1")
        assert v2.endswith(".2")
