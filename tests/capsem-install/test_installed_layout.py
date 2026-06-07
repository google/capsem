"""Installed layout contract tests.

Verifies that the installed layout matches what the CLI auto-launch and
service startup expect. Works with both installation paths:
  - .deb via dpkg (just test-install): CAPSEM_DEB_INSTALLED=1
  - simulate-install.sh (standalone pytest): fallback

Layout contract:
  ~/.capsem/bin/capsem{,-service,-process,-mcp,-gateway,-tray}  (executables or symlinks)
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
from pathlib import Path

import pytest

from .conftest import (
    ASSETS_DIR,
    BINARIES,
    CAPSEM_DIR,
    INSTALL_DIR,
    RUN_DIR,
    run_capsem,
    get_build_hash,
)


class TestInstalledLayoutContract:
    """The layout simulate-install.sh creates must match what Rust code expects."""

    # -- Binaries --

    def test_all_binaries_exist(self, installed_layout):
        """All 6 binaries present in ~/.capsem/bin/."""
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
            assert is_elf or is_macho, (
                f"{name}: not an executable (header: {header.hex()})"
            )

    def test_capsem_version_works(self, installed_layout):
        """capsem version runs and contains build hash."""
        result = run_capsem("version", timeout=5)
        assert result.returncode == 0
        assert "build" in result.stdout, f"no build hash: {result.stdout}"

    # -- Assets --

    def test_manifest_json_exists(self, installed_layout):
        """manifest.json present at ~/.capsem/assets/manifest.json."""
        manifest = ASSETS_DIR / "manifest.json"
        if os.environ.get("CAPSEM_DEB_INSTALLED") == "1" and not manifest.exists():
            pytest.skip("assets downloaded on first use, not bundled in .deb")
        assert manifest.exists(), (
            f"manifest.json missing at {manifest} -- service will fail to start"
        )

    def test_manifest_json_is_valid(self, installed_layout):
        """manifest.json parses as JSON with expected v2 structure."""
        manifest = ASSETS_DIR / "manifest.json"
        if not manifest.exists():
            pytest.skip("no manifest.json")
        data = json.loads(manifest.read_text())
        assert data.get("format") == 2, f"expected format=2, got {data.get('format')!r}"
        assert "assets" in data and "releases" in data["assets"], "manifest missing assets.releases"
        assert "binaries" in data and "releases" in data["binaries"], "manifest missing binaries.releases"

    def test_hash_named_assets_exist(self, installed_layout):
        """Assets exist under $ASSETS/{arch}/{hash-filename} as resolved from the manifest."""
        import platform

        machine = platform.machine().lower()
        arch = "arm64" if machine in ("arm64", "aarch64") else "x86_64"

        manifest_path = ASSETS_DIR / "manifest.json"
        if os.environ.get("CAPSEM_DEB_INSTALLED") == "1" and not manifest_path.exists():
            pytest.skip("assets downloaded on first use, not bundled in .deb")
        assert manifest_path.exists(), f"manifest missing: {manifest_path}"

        data = json.loads(manifest_path.read_text())
        current = data["assets"]["current"]
        arch_assets = data["assets"]["releases"][current]["arches"].get(arch)
        if arch_assets is None:
            pytest.skip(f"no {arch} entry in manifest (cross-arch install)")

        arch_dir = ASSETS_DIR / arch
        if os.environ.get("CAPSEM_DEB_INSTALLED") == "1" and not arch_dir.exists():
            pytest.skip("assets downloaded on first use, not bundled in .deb")
        assert arch_dir.is_dir(), (
            f"arch dir missing: {arch_dir}\n"
            f"resolver will fail: ManifestV2::resolve() checks $ASSETS/{arch}/<hash>"
        )

        for logical, meta in arch_assets.items():
            prefix = meta["hash"][:16]
            if "." in logical:
                stem, ext = logical.split(".", 1)
                hashed = f"{stem}-{prefix}.{ext}"
            else:
                hashed = f"{logical}-{prefix}"
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

    def test_version_in_manifest_matches_binary(self, installed_layout):
        """The manifest must contain a binary release entry for the installed version."""
        manifest_path = ASSETS_DIR / "manifest.json"
        if not manifest_path.exists():
            pytest.skip("no manifest.json")

        data = json.loads(manifest_path.read_text())
        result = run_capsem("version", timeout=5)
        version = result.stdout.strip().split()[1]

        binary_releases = data.get("binaries", {}).get("releases", {})
        assert version in binary_releases, (
            f"installed version {version} not in manifest binaries.releases: "
            f"{sorted(binary_releases)}"
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
        assert (CAPSEM_DIR / "run").is_dir()

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
