"""Profile-lane release graph guards."""

from __future__ import annotations

from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]
RELEASE_GRAPH = PROJECT_ROOT / "crates" / "capsem-admin" / "src" / "release_graph.rs"


def test_profile_json_has_min_capsem_not_current_binary() -> None:
    source = RELEASE_GRAPH.read_text(encoding="utf-8")

    assert "pub struct ProfileDocument" in source
    assert "pub min_capsem_version: Option<String>" in source
    profile_document = source.split("pub struct ProfileDocument", maxsplit=1)[1].split(
        "pub struct SoftwareInventoryRow", maxsplit=1
    )[0]
    assert "current_binary" not in profile_document
    assert "current_assets" not in profile_document
    assert "pub struct SoftwareInventoryRow" in source
    assert "pub struct ProfileConfigRef" in source
    assert "pub struct ProfileArchitectureImages" in source
    assert "pub struct ProfileImageArtifactRef" in source
    assert "profile_json_ownership_rejects_current_binary_and_assets" in source
