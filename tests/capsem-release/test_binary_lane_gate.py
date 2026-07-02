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
