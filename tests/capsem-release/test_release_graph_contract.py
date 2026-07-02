"""Release graph contract tests.

These tests guard the typed graph vocabulary before the release machinery grows
around it. The executable Rust tests prove serde behavior; these Python checks
keep the source contract visible to the release CI lane.
"""

from __future__ import annotations

from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]
RELEASE_GRAPH = PROJECT_ROOT / "crates" / "capsem-admin" / "src" / "release_graph.rs"


def _source() -> str:
    return RELEASE_GRAPH.read_text(encoding="utf-8")


def test_status_enum_rejects_unknown_values() -> None:
    source = _source()

    assert "pub enum Status" in source
    for variant in ("Current", "Supported", "Deprecated", "Revoked"):
        assert variant in source
    assert "Removed" not in source
    assert "release_graph_enums_reject_unknown_status_values" in source


def test_manifest_record_uses_version_not_schema_version() -> None:
    source = _source()

    assert "pub struct ManifestRecord" in source
    manifest_record = source.split("pub struct ManifestRecord", maxsplit=1)[1].split(
        "pub struct ChannelRecord", maxsplit=1
    )[0]
    assert "pub version: String" in manifest_record
    assert "schema_version" not in manifest_record
    assert "release_graph_manifest_records_use_version_not_schema_version" in source


def test_channels_json_lists_all_manifest_records() -> None:
    source = _source()

    assert "pub struct ChannelsCatalog" in source
    assert "pub channels: BTreeMap<String, ChannelRecord>" in source
    assert "pub manifests: Vec<ManifestRecord>" in source
    assert "release_graph_channels_catalog_lists_manifest_records" in source
    assert "release_graph_channels_catalog_rejects_duplicate_manifest_versions" in source


def test_graph_verifier_rejects_tampered_profile_ref() -> None:
    source = _source()

    assert "pub fn verify_bytes" in source
    assert "sha256 mismatch" in source
    assert "blake3 mismatch" in source
    assert "release_graph_digest_verifier_rejects_tampered_profile_ref" in source


def test_revoked_manifest_is_listed_but_not_selectable() -> None:
    source = _source()

    assert "pub fn select_manifest" in source
    assert "Status::Revoked => 255" in source
    assert "release_graph_revoked_manifest_is_listed_but_not_selectable" in source
