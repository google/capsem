from __future__ import annotations

import json
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]
FIXTURE = (
    PROJECT_ROOT
    / "tests"
    / "capsem-release"
    / "fixtures"
    / "release-graph-stable-nightly.json"
)


def test_package_and_machine_architecture_vocabularies_never_cross() -> None:
    manifest = json.loads(FIXTURE.read_text())

    for channel in manifest["manifests"].values():
        for release in channel.values():
            for package in release["packages"]:
                architecture = package["architecture"]
                assert architecture in {"amd64", "arm64"}, package
                assert all(
                    binary["architecture"] == architecture
                    for binary in package["binaries"]
                ), package
                if package["kind"] == "debian_package":
                    assert package["platform"] == "linux"
                    assert package["name"].endswith(f"_{architecture}.deb")
                    assert "_x86_64.deb" not in package["name"]
                else:
                    assert package["kind"] == "macos_pkg"
                    assert package["platform"] == "macos"
                    assert architecture == "arm64"
                    assert package["name"].endswith(".pkg")

            for profile in release["profiles"].values():
                for architecture in profile["architectures"]:
                    assert architecture["architecture"] in {"arm64", "x86_64"}
                    assert architecture["architecture"] != "amd64"


def test_rust_graph_uses_distinct_typed_architecture_domains() -> None:
    core = (
        PROJECT_ROOT / "crates" / "capsem-core" / "src" / "asset_manager.rs"
    ).read_text()
    source = (
        PROJECT_ROOT / "crates" / "capsem-admin" / "src" / "release_graph.rs"
    ).read_text()
    main = (PROJECT_ROOT / "crates" / "capsem-admin" / "src" / "main.rs").read_text()
    updater = (PROJECT_ROOT / "crates" / "capsem" / "src" / "update.rs").read_text()

    assert "pub enum PackageArchitecture {" in core
    assert "Amd64," in core
    assert "pub use capsem_core::asset_manager::{Architecture, PackageArchitecture};" in source
    package_row = source.split("pub struct PackageInventoryRow", maxsplit=1)[1].split(
        "}", maxsplit=1
    )[0]
    binary_row = source.split("pub struct BinaryInventoryRow", maxsplit=1)[1].split(
        "}", maxsplit=1
    )[0]
    software_row = source.split("pub struct SoftwareInventoryRow", maxsplit=1)[1].split(
        "}", maxsplit=1
    )[0]
    assert "pub architecture: PackageArchitecture" in package_row
    assert "pub architecture: PackageArchitecture" in binary_row
    assert "pub architecture: Architecture" in software_row
    assert "fn package_architecture_for_name(name: &str) -> String" not in main
    assert 'name.contains("amd64")' not in main
    assert "architecture: PackageArchitecture" in updater
    assert "architecture: Architecture" in updater
    assert "fn deb_graph_arch" not in updater


def test_public_linux_package_consumers_use_debian_identity() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "release.yaml").read_text()
    validator = (
        PROJECT_ROOT / "scripts" / "check-public-binary-release.py"
    ).read_text()

    assert "linux/x86_64 package" not in workflow
    assert "package.get('architecture') == 'x86_64'" not in workflow
    assert 'RequiredPackage("linux", "x86_64", "debian_package")' not in validator
    assert 'package.get("architecture") != "x86_64"' not in validator
    assert 'package.get("architecture") == "x86_64"' not in validator
