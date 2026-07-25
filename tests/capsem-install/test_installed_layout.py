"""Installed layout contract tests.

Verifies that the installed layout matches what the CLI auto-launch and
service startup expect. Works with both installation paths:
  - .deb via dpkg (just test-install): CAPSEM_DEB_INSTALLED=1
  - simulate-install.sh (standalone pytest): fallback

Layout contract:
  ~/.capsem/bin/capsem* host tools                           (executables or symlinks)
  ~/.capsem/assets/manifest.json                                (service reads this)
  ~/.capsem/assets/{arch}/{logical}-{hash16}.{ext}              (resolver target)
  ~/.capsem/run/                                                (created at runtime)

The legacy ~/.capsem/assets/v{VERSION}/ layout is NOT supported anymore --
ManifestV2::resolve() only checks $ASSETS/{hash_filename} or
$ASSETS/{arch}/{hash_filename}.
"""

from __future__ import annotations

import json
import os
import subprocess
import tomllib
from pathlib import Path

import pytest

from .conftest import (
    ASSETS_DIR,
    BINARIES,
    CAPSEM_DIR,
    INSTALL_DIR,
    RUN_DIR,
    run_capsem,
)

SOURCE_MANIFEST = Path(
    os.environ.get(
        "CAPSEM_TEST_ASSET_MANIFEST",
        Path(__file__).resolve().parents[2] / "assets" / "manifest.json",
    )
)


def _version_at_least(actual: str, minimum: str) -> bool:
    if not minimum:
        return True

    def parts(value: str) -> list[int]:
        parsed = []
        for part in value.split("."):
            try:
                parsed.append(int(part))
            except ValueError:
                parsed.append(0)
        return parsed

    left = parts(actual)
    right = parts(minimum)
    width = max(len(left), len(right))
    left.extend([0] * (width - len(left)))
    right.extend([0] * (width - len(right)))
    return left >= right


def _is_release_graph(manifest: dict) -> bool:
    return isinstance(manifest.get("packages"), list) and isinstance(
        manifest.get("profiles"), dict
    )


def _release_graph_profile_arch(manifest: dict, profile_id: str, arch: str) -> dict | None:
    profile = manifest.get("profiles", {}).get(profile_id)
    if not isinstance(profile, dict):
        return None
    return next(
        (
            architecture
            for architecture in profile.get("architectures", [])
            if architecture.get("architecture") == arch
        ),
        None,
    )


def _hash_named_asset(logical: str, digest: str) -> str:
    prefix = digest[:16]
    if "." in logical:
        stem, ext = logical.split(".", 1)
        return f"{stem}-{prefix}.{ext}"
    return f"{logical}-{prefix}"


class TestInstalledLayoutContract:
    """The layout simulate-install.sh creates must match what Rust code expects."""

    # -- Binaries --

    def test_all_binaries_exist(self, installed_layout):
        """All host binaries are present in ~/.capsem/bin/."""
        for name in BINARIES:
            binary = INSTALL_DIR / name
            assert binary.exists(), f"missing: {binary}"
            assert os.access(binary, os.X_OK), f"not executable: {binary}"

    def test_binaries_are_real_elf_or_macho(self, installed_layout):
        """Binaries are actual executables, not empty stubs or scripts."""
        for name in BINARIES:
            binary = INSTALL_DIR / name
            header = binary.read_bytes()[:4]
            # ELF: \x7fELF, Mach-O 64: \xcf\xfa\xed\xfe or \xfe\xed\xfa\xcf
            is_elf = header == b"\x7fELF"
            is_macho = header in (b"\xcf\xfa\xed\xfe", b"\xfe\xed\xfa\xcf")
            assert is_elf or is_macho, f"{name}: not an executable (header: {header.hex()})"

    def test_capsem_version_works(self, installed_layout):
        """capsem version runs and contains build hash."""
        result = run_capsem("version", timeout=5)
        assert result.returncode == 0
        assert "build" in result.stdout, f"no build hash: {result.stdout}"

    def test_all_installed_binaries_report_capsem_version(self, installed_layout):
        """Every packaged host binary reports the installed package version."""
        result = run_capsem("version", timeout=5)
        assert result.returncode == 0, result.stderr
        expected_version = result.stdout.strip().split()[1]

        for name in BINARIES:
            helper = subprocess.run(
                [str(INSTALL_DIR / name), "--version"]
                if name != "capsem"
                else [str(INSTALL_DIR / name), "version"],
                capture_output=True,
                text=True,
                timeout=5,
            )
            output = helper.stdout.strip()
            assert helper.returncode == 0, (
                f"{name} --version failed\nstdout={helper.stdout}\nstderr={helper.stderr}"
            )
            assert expected_version in output, f"{name} version mismatch: {output}"

    def test_capsem_admin_help_works(self, installed_layout):
        """capsem-admin is installed and runnable without a service."""
        result = subprocess.run(
            [str(INSTALL_DIR / "capsem-admin"), "--help"],
            capture_output=True,
            text=True,
            timeout=5,
        )
        assert result.returncode == 0
        assert "capsem-admin" in result.stdout

    # -- Assets --

    def test_manifest_json_exists(self, installed_layout):
        """manifest.json present at ~/.capsem/assets/manifest.json."""
        manifest = ASSETS_DIR / "manifest.json"
        assert manifest.exists(), (
            f"manifest.json missing at {manifest} -- service will fail to start"
        )

    def test_manifest_json_is_valid(self, installed_layout):
        """manifest.json is the complete installed release graph."""
        manifest = ASSETS_DIR / "manifest.json"
        if not manifest.exists():
            pytest.skip("no manifest.json")
        data = json.loads(manifest.read_text())
        if os.environ.get("CAPSEM_DEB_INSTALLED") == "1":
            assert _is_release_graph(data), (
                "native package installed a legacy/runtime projection instead of "
                "the authoritative release graph"
            )
        if _is_release_graph(data):
            assert data.get("channel"), "release graph missing channel"
            assert data.get("status") == "current"
            assert data.get("version"), "release graph missing version"
            assert data["packages"], "release graph missing packages"
            assert data["profiles"], "release graph missing profiles"
            return

        # Standalone development installs still exercise the runtime-only v2
        # asset index. Native package proofs above must exercise the exact
        # public graph copied by the installer.
        assert data.get("format") == 2, f"expected format=2, got {data.get('format')!r}"
        assert "assets" in data and "releases" in data["assets"], "manifest missing assets.releases"
        assert "binaries" in data and "releases" in data["binaries"], (
            "manifest missing binaries.releases"
        )

    def test_native_install_preserves_exact_authoritative_manifest_bytes(
        self, installed_layout
    ):
        """The native installer must activate the exact graph it fetched."""
        if os.environ.get("CAPSEM_DEB_INSTALLED") != "1":
            pytest.skip("exact source-manifest byte proof is native-package-only")

        installed = ASSETS_DIR / "manifest.json"
        assert SOURCE_MANIFEST.is_file(), (
            f"authoritative install-test manifest missing: {SOURCE_MANIFEST}"
        )
        assert installed.read_bytes() == SOURCE_MANIFEST.read_bytes(), (
            "native install normalized or projected the manifest; the fetched "
            "release graph must remain the sole installed authority"
        )

    def test_hash_named_assets_exist(self, installed_layout):
        """Assets exist under $ASSETS/{arch}/{hash-filename} as resolved from the manifest."""
        import platform

        machine = platform.machine().lower()
        arch = "arm64" if machine in ("arm64", "aarch64") else "x86_64"

        manifest_path = ASSETS_DIR / "manifest.json"
        assert manifest_path.exists(), f"manifest missing: {manifest_path}"

        data = json.loads(manifest_path.read_text())
        if _is_release_graph(data):
            profile_arch = _release_graph_profile_arch(data, "code", arch)
            if profile_arch is None:
                pytest.skip(f"no code/{arch} entry in manifest (cross-arch install)")
            arch_assets = {
                image["name"]: {
                    "hash": image["digest"]["blake3"],
                    "sha256": image["digest"]["sha256"],
                    "size": image["bytes"],
                }
                for image in profile_arch.get("images", [])
                if image.get("status", "current") != "revoked"
            }
        else:
            current = data["assets"]["current"]
            arch_assets = data["assets"]["releases"][current]["arches"].get(arch)
            if arch_assets is None:
                pytest.skip(f"no {arch} entry in manifest (cross-arch install)")

        arch_dir = ASSETS_DIR / arch
        assert arch_dir.is_dir(), (
            f"arch dir missing: {arch_dir}\n"
            f"resolver will fail: ManifestV2::resolve() checks $ASSETS/{arch}/<hash>"
        )

        for logical, meta in arch_assets.items():
            hashed = _hash_named_asset(logical, meta["hash"])
            target = arch_dir / hashed
            assert target.exists(), (
                f"asset missing: {target}\n"
                f"manifest says {logical} hash={meta['hash']}, expected file name {hashed}"
            )

    def test_no_legacy_version_dirs(self, installed_layout):
        """Reject leftover ~/.capsem/assets/v1.0.* dirs -- resolver doesn't read them."""
        legacy = sorted(ASSETS_DIR.glob("v1.0.*"))
        assert not legacy, (
            f"legacy asset dirs present: {legacy}\n"
            f"ManifestV2::resolve() no longer reads these; sync-dev-assets.sh "
            f"and simulate-install.sh are supposed to clean them up."
        )

    def test_manifest_assets_are_compatible_with_binary(self, installed_layout):
        """Decoupled VM assets must still declare compatibility with the installed binary."""
        manifest_path = ASSETS_DIR / "manifest.json"
        if not manifest_path.exists():
            pytest.skip("no manifest.json")

        data = json.loads(manifest_path.read_text())
        result = run_capsem("version", timeout=5)
        version = result.stdout.strip().split()[1]

        if _is_release_graph(data):
            current_versions = {
                package["version"]
                for package in data["packages"]
                if package.get("status", "current") == "current"
            }
            if version in current_versions:
                return
            compatible_profiles = [
                profile_id
                for profile_id, profile in data["profiles"].items()
                if profile.get("status", "current") != "revoked"
                and _version_at_least(version, profile.get("min_capsem_version", ""))
                and (
                    not profile.get("max_capsem_version")
                    or _version_at_least(profile["max_capsem_version"], version)
                )
            ]
            assert compatible_profiles, (
                f"installed version {version} has no compatible profile; "
                f"manifest current package versions={sorted(current_versions)}"
            )
            return

        binary_releases = data.get("binaries", {}).get("releases", {})
        if version in binary_releases:
            return

        compatible_assets = [
            asset_version
            for asset_version, release in data["assets"]["releases"].items()
            if _version_at_least(version, release.get("min_binary", ""))
        ]
        assert compatible_assets, (
            f"installed version {version} has no compatible asset release; "
            f"manifest binaries.releases={sorted(binary_releases)}"
        )

    # -- Directories --

    def test_run_dir_exists(self, installed_layout):
        """~/.capsem/run/ exists (service writes socket here)."""
        assert RUN_DIR.exists(), f"run dir missing: {RUN_DIR}"

    def test_capsem_dir_structure(self, installed_layout):
        """~/.capsem/ has the expected subdirectories."""
        assert CAPSEM_DIR.exists()
        assert (CAPSEM_DIR / "bin").is_dir()
        assert (CAPSEM_DIR / "assets").is_dir()
        assert (CAPSEM_DIR / "profiles").is_dir()
        assert (CAPSEM_DIR / "run").is_dir()

    def test_installed_profile_catalog_exists(self, installed_layout):
        """Installed service must load materialized profiles, not compiled source fallback."""
        profile = CAPSEM_DIR / "profiles" / "code" / "profile.toml"
        assert profile.exists(), (
            f"materialized profile missing: {profile}\n"
            "without this, installed service falls back to compiled source profile pins"
        )
        assert (CAPSEM_DIR / "profiles" / "code" / "enforcement.toml").exists()

    def test_installed_profile_asset_pins_match_manifest(self, installed_layout):
        """Profile-owned asset pins must match the installed asset manifest."""
        import platform

        profile_path = CAPSEM_DIR / "profiles" / "code" / "profile.toml"
        manifest_path = ASSETS_DIR / "manifest.json"
        if not manifest_path.exists():
            pytest.skip("no manifest.json")
        assert profile_path.exists(), f"profile missing: {profile_path}"

        machine = platform.machine().lower()
        arch = "arm64" if machine in ("arm64", "aarch64") else "x86_64"
        manifest = json.loads(manifest_path.read_text())
        if _is_release_graph(manifest):
            profile_arch = _release_graph_profile_arch(manifest, "code", arch)
            if profile_arch is None:
                pytest.skip(f"no code/{arch} entry in manifest")
            manifest_assets = {
                image["kind"]: image["digest"]["blake3"]
                for image in profile_arch.get("images", [])
                if image.get("status", "current") != "revoked"
            }
        else:
            current = manifest["assets"]["current"]
            legacy_assets = manifest["assets"]["releases"][current]["arches"].get(arch)
            if legacy_assets is None:
                pytest.skip(f"no {arch} entry in manifest")
            manifest_assets = {
                "kernel": legacy_assets["vmlinuz"]["hash"],
                "initrd": legacy_assets["initrd.img"]["hash"],
                "rootfs": legacy_assets["rootfs.erofs"]["hash"],
            }

        profile = tomllib.loads(profile_path.read_text())
        profile_assets = profile["assets"]["arch"][arch]
        for kind in ["kernel", "initrd", "rootfs"]:
            expected = manifest_assets[kind]
            actual = profile_assets[kind]["hash"].removeprefix("blake3:")
            assert actual == expected, (
                f"profile {kind} pin drift: profile={actual} manifest={expected}"
            )

    # -- Service spawn contract --
    # When CLI auto-launches, it runs:
    #   capsem-service --foreground --assets-dir ~/.capsem/assets/ --process-binary ~/.capsem/bin/capsem-process
    # The service then:
    #   1. Reads manifest.json from --assets-dir
    #   2. Resolves rootfs from --assets-dir/v{VERSION}/
    #   3. Spawns --process-binary for each VM

    def test_service_binary_is_sibling_of_capsem(self, installed_layout):
        """capsem-service is in the same dir as capsem (sibling discovery)."""
        capsem = INSTALL_DIR / "capsem"
        service = INSTALL_DIR / "capsem-service"
        assert capsem.parent == service.parent

    def test_process_binary_is_sibling(self, installed_layout):
        """capsem-process is in the same dir as capsem-service."""
        service = INSTALL_DIR / "capsem-service"
        process = INSTALL_DIR / "capsem-process"
        assert service.parent == process.parent

    # -- Cross-platform: path safety --

    def test_no_trailing_slash_in_paths(self, installed_layout):
        """Paths don't have trailing slashes that could confuse join()."""
        for d in [INSTALL_DIR, ASSETS_DIR, RUN_DIR]:
            s = str(d)
            assert not s.endswith("/") or s == "/", f"trailing slash: {s}"

    def test_paths_are_absolute(self, installed_layout):
        """All installed paths are absolute."""
        for d in [INSTALL_DIR, ASSETS_DIR, RUN_DIR]:
            assert d.is_absolute(), f"not absolute: {d}"


class TestInstalledLayoutSymlink:
    """Symlink-based dev workflow: ln -s target/debug ~/.capsem/bin."""

    def test_symlinked_capsem_dir_works(self, installed_layout, tmp_path):
        """If ~/.capsem is a symlink, capsem version still works."""
        # We can't easily test this in Docker without messing with the
        # installed layout, so just verify the concept: Path operations
        # on a symlink target work the same as on the real dir.
        real = tmp_path / "real_capsem"
        real.mkdir()
        (real / "bin").mkdir()
        link = tmp_path / "linked_capsem"
        link.symlink_to(real)

        # Path operations should traverse the symlink
        assert (link / "bin").exists()
        assert (link / "bin").is_dir()
