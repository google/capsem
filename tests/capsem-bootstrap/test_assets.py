"""Asset manifest, hashes, and architecture verification.

These tests do NOT boot VMs -- they validate the build artifacts.
"""

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).parent.parent.parent
ASSETS_DIR = PROJECT_ROOT / "assets"

pytestmark = pytest.mark.bootstrap


def test_manifest_schema_accepts_release_graph() -> None:
    manifest = {
        "version": "1.6.1",
        "channel": "nightly",
        "status": "current",
        "packages": [{"status": "current"}],
        "profiles": {
            "code": {
                "status": "current",
                "architectures": [{"architecture": _host_arch()}],
            }
        },
    }

    assert _manifest_schema(manifest) == "release_graph"


@pytest.mark.parametrize(
    "manifest",
    [
        {},
        {"format": 2, "assets": {}, "binaries": {}},
        {"channel": "nightly", "packages": [], "profiles": {}},
        {"channel": ["nightly"], "packages": [{}], "profiles": {"code": {}}},
    ],
)
def test_manifest_schema_rejects_incomplete_documents(manifest: dict) -> None:
    with pytest.raises(AssertionError):
        _manifest_schema(manifest)


def test_bootstrap_manifest_contract_accepts_a_selected_release_graph(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    cargo_version = next(
        line.split("=", maxsplit=1)[1].strip().strip('"')
        for line in (PROJECT_ROOT / "Cargo.toml").read_text().splitlines()
        if line.strip().startswith("version") and "=" in line
    )
    (tmp_path / "manifest.json").write_text(
        json.dumps(
            {
                "version": "1",
                "channel": "nightly",
                "status": "current",
                "packages": [
                    {
                        "version": cargo_version,
                        "status": "current",
                    }
                ],
                "profiles": {
                    "code": {
                        "status": "current",
                        "architectures": [{"architecture": _host_arch()}],
                    }
                },
            }
        ),
        encoding="utf-8",
    )
    monkeypatch.setattr(sys.modules[__name__], "ASSETS_DIR", tmp_path)

    manifest_contract = TestManifest()
    manifest_contract.test_manifest_valid_json()
    manifest_contract.test_manifest_binary_version_matches_cargo()
    manifest_contract.test_manifest_has_host_arch()


def _host_arch():
    return "arm64" if os.uname().machine == "arm64" else "x86_64"


def _manifest_schema(data: dict) -> str:
    if data.get("format") == 2:
        assets = data.get("assets")
        binaries = data.get("binaries")
        assert isinstance(assets, dict) and isinstance(assets.get("releases"), dict)
        assert isinstance(assets.get("current"), str) and assets["current"]
        assert isinstance(binaries, dict) and isinstance(
            binaries.get("releases"), dict
        )
        return "legacy_v2"

    assert isinstance(data.get("version"), str) and data["version"]
    assert isinstance(data.get("channel"), str) and data["channel"]
    packages = data.get("packages")
    profiles = data.get("profiles")
    assert isinstance(packages, list) and packages
    assert any(
        isinstance(package, dict) and package.get("status") == "current"
        for package in packages
    )
    assert isinstance(profiles, dict) and profiles
    active_profiles = [
        profile
        for profile in profiles.values()
        if isinstance(profile, dict) and profile.get("status") != "revoked"
    ]
    assert active_profiles
    assert all(
        isinstance(profile.get("architectures"), list)
        and profile["architectures"]
        for profile in active_profiles
    )
    return "release_graph"


class TestManifest:

    def test_manifest_exists(self):
        manifest = ASSETS_DIR / "manifest.json"
        assert manifest.exists(), f"manifest.json not found at {manifest}"

    def test_manifest_valid_json(self):
        manifest = ASSETS_DIR / "manifest.json"
        if not manifest.exists():
            pytest.skip("No manifest.json")
        data = json.loads(manifest.read_text())
        _manifest_schema(data)

    def test_manifest_binary_version_matches_cargo(self):
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

        schema = _manifest_schema(data)
        if schema == "legacy_v2":
            # Cargo version = BINARY version (asset version evolves independently).
            binary_releases = data["binaries"]["releases"]
            assert cargo_version in binary_releases, (
                f"Cargo version {cargo_version} not in manifest binaries.releases "
                f"(have: {sorted(binary_releases)})"
            )
        else:
            package_versions = {
                package.get("version")
                for package in data["packages"]
                if isinstance(package, dict) and package.get("status") == "current"
            }
            assert cargo_version in package_versions, (
                f"Cargo version {cargo_version} not selected by current manifest packages "
                f"(have: {sorted(str(version) for version in package_versions)})"
            )

    def test_manifest_has_host_arch(self):
        manifest = ASSETS_DIR / "manifest.json"
        if not manifest.exists():
            pytest.skip("No manifest.json")
        data = json.loads(manifest.read_text())
        arch = _host_arch()
        schema = _manifest_schema(data)
        if schema == "legacy_v2":
            current = data["assets"]["current"]
            arches = data["assets"]["releases"].get(current, {}).get("arches", {})
            assert arch in arches, (
                f"No {arch} entry in manifest for asset version {current} "
                f"(have: {sorted(arches)})"
            )
        else:
            missing = []
            for profile_id, profile in data["profiles"].items():
                if not isinstance(profile, dict) or profile.get("status") == "revoked":
                    continue
                architectures = {
                    row.get("architecture")
                    for row in profile["architectures"]
                    if isinstance(row, dict)
                }
                if arch not in architectures:
                    missing.append(profile_id)
            assert not missing, (
                f"Active profiles missing {arch} release artifacts: {sorted(missing)}"
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
        rootfs = ASSETS_DIR / arch / "rootfs.erofs"
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
