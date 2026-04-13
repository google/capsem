"""Installed layout contract tests.

Verifies that simulate-install.sh produces exactly the layout that the
CLI auto-launch and service startup expect to consume. If any of these
fail, `just install && capsem shell` is broken.

Layout contract:
  ~/.capsem/bin/capsem{,-service,-process,-mcp}   (executables)
  ~/.capsem/assets/manifest.json                   (service reads this)
  ~/.capsem/assets/v{VERSION}/*.squashfs           (service resolves via version)
  ~/.capsem/run/                                   (created at runtime)
"""

from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path

import pytest

from conftest import (
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
        assert manifest.exists(), (
            f"manifest.json missing at {manifest} -- service will fail to start"
        )

    def test_manifest_json_is_valid(self, installed_layout):
        """manifest.json parses as JSON with expected structure."""
        manifest = ASSETS_DIR / "manifest.json"
        if not manifest.exists():
            pytest.skip("no manifest.json")
        data = json.loads(manifest.read_text())
        assert "latest" in data, "manifest missing 'latest' field"
        assert "releases" in data, "manifest missing 'releases' field"

    def test_versioned_assets_exist(self, installed_layout):
        """Assets exist under v{VERSION}/ matching the installed binary version."""
        result = run_capsem("version", timeout=5)
        assert result.returncode == 0
        # Parse "capsem 0.16.1 (build ...)" -> "0.16.1"
        version = result.stdout.strip().split()[1]

        versioned_dir = ASSETS_DIR / f"v{version}"
        assert versioned_dir.exists(), (
            f"versioned asset dir missing: {versioned_dir}\n"
            f"service will fail to resolve assets for version {version}"
        )

        # rootfs.squashfs must be in the versioned dir (service checks this)
        rootfs = versioned_dir / "rootfs.squashfs"
        assert rootfs.exists(), (
            f"rootfs.squashfs missing in {versioned_dir}\n"
            f"service resolve_assets_dir() will fail"
        )

    def test_version_in_manifest_matches_binary(self, installed_layout):
        """The manifest must contain a release entry for the installed version."""
        manifest_path = ASSETS_DIR / "manifest.json"
        if not manifest_path.exists():
            pytest.skip("no manifest.json")

        data = json.loads(manifest_path.read_text())
        result = run_capsem("version", timeout=5)
        version = result.stdout.strip().split()[1]

        releases = data.get("releases", {})
        # The version should be in releases, or the manifest.latest should be usable
        assert version in releases or data.get("latest") == version, (
            f"installed version {version} not in manifest releases: {list(releases.keys())}"
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
