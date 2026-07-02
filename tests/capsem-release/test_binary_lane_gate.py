"""Binary-lane release graph guards."""

from __future__ import annotations

from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]
RELEASE_GRAPH = PROJECT_ROOT / "crates" / "capsem-admin" / "src" / "release_graph.rs"


def test_package_rows_are_not_binary_rows() -> None:
    source = RELEASE_GRAPH.read_text(encoding="utf-8")

    assert "pub struct PackageInventoryRow" in source
    assert "pub struct BinaryInventoryRow" in source
    assert "pub packages: Vec<PackageInventoryRow>" in source
    assert "pub binaries: Vec<BinaryInventoryRow>" in source
    assert "pub package: String" in source
    assert "pub install_path: String" in source
    assert "pub sbom_component_ref: String" in source
    assert "package_inventory_rows_are_separate_from_binary_rows" in source


def test_every_packaged_executable_has_hashes_and_sbom_ref() -> None:
    source = RELEASE_GRAPH.read_text(encoding="utf-8")

    assert "pub struct PackagedExecutableFile" in source
    assert "pub fn executable_inventory_from_package_files" in source
    assert "pub fn verify_package_contents_match_binary_inventory" in source
    assert "format!(\"{:x}\", Sha256::digest(&file.bytes))" in source
    assert "blake3::hash(&file.bytes)" in source
    assert "sbom_component_refs" in source
    assert "missing SBOM component reference" in source
    assert (
        "executable_inventory_records_every_packaged_binary_with_hashes_and_sbom_refs"
        in source
    )
    assert "executable_inventory_rejects_missing_sbom_component_ref" in source
    assert "executable_inventory_matches_macos_and_deb_package_contents" in source
    assert "executable_inventory_rejects_package_content_hash_drift" in source
