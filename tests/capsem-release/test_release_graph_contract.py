"""Release graph contract tests.

These tests guard the typed graph vocabulary before the release machinery grows
around it. The executable Rust tests prove serde behavior; these Python checks
keep the source contract visible to the release CI lane.
"""

from __future__ import annotations

from pathlib import Path
import json


PROJECT_ROOT = Path(__file__).resolve().parents[2]
RELEASE_GRAPH = PROJECT_ROOT / "crates" / "capsem-admin" / "src" / "release_graph.rs"
FIXTURE_GRAPH = (
    PROJECT_ROOT
    / "tests"
    / "capsem-release"
    / "fixtures"
    / "release-graph-stable-nightly.json"
)


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


def test_fixture_has_stable_and_nightly() -> None:
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    assert sorted(graph["channels"]) == ["nightly", "stable"]
    assert {
        manifest["status"]
        for channel in graph["channels"].values()
        for manifest in channel["manifests"]
    } == {"current", "supported", "deprecated", "revoked"}
    assert graph["channels"]["stable"]["manifests"][0]["version"] == "1.0.2"
    assert graph["channels"]["nightly"]["manifests"][0]["version"] == "1.0.2"

    stable = graph["manifests"]["stable"]["1.0.2"]
    nightly = graph["manifests"]["nightly"]["1.0.2"]
    assert stable["packages"][0]["name"] == "Capsem-1.4.0.pkg"
    assert nightly["packages"][0]["name"] == "Capsem-1.5.0-nightly.20260702.pkg"
    assert "binaries" not in stable
    stable_binary_refs = {
        binary["name"]: binary["sbom_component_ref"]
        for binary in stable["packages"][0]["binaries"]
    }
    assert stable_binary_refs == {
        "capsem": "SPDXRef-File-capsem",
        "capsem-admin": "SPDXRef-File-capsem-admin",
        "capsem-app": "SPDXRef-File-capsem-app",
        "capsem-gateway": "SPDXRef-File-capsem-gateway",
        "capsem-mcp": "SPDXRef-File-capsem-mcp",
        "capsem-mcp-aggregator": "SPDXRef-File-capsem-mcp-aggregator",
        "capsem-mcp-builtin": "SPDXRef-File-capsem-mcp-builtin",
        "capsem-process": "SPDXRef-File-capsem-process",
        "capsem-service": "SPDXRef-File-capsem-service",
        "capsem-tray": "SPDXRef-File-capsem-tray",
        "capsem-tui": "SPDXRef-File-capsem-tui",
    }
    assert "-nightly." in nightly["profiles"]["co-work"]["revision"]
    assert "-stable." in stable["profiles"]["co-work"]["revision"]
    assert nightly["profiles"]["co-work"]["min_capsem_version"] == "1.4.0"
    assert (
        nightly["profiles"]["co-work"]["architectures"][0]["evidence"][0]["kind"]
        == "abom"
    )


def test_ledger_is_derived_not_authoritative() -> None:
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    source = _source()

    assert "ledger" not in graph
    assert "pub struct ReleaseLedger" in source
    assert "pub fn derive(" in source
    stable = graph["manifests"]["stable"]["1.0.2"]
    derived_kinds = {
        "manifest" if graph["channels"]["stable"]["manifests"] else "",
        "package" if stable["packages"] else "",
        "binary" if stable["packages"][0]["binaries"] else "",
        "profile" if stable["profiles"] else "",
        "profile_image" if stable["profiles"]["co-work"]["architectures"] else "",
    }
    assert derived_kinds == {"manifest", "package", "binary", "profile", "profile_image"}


def test_health_json_not_release_truth() -> None:
    release_site_loader = (
        PROJECT_ROOT / "release-site" / "src" / "lib" / "release-data.ts"
    ).read_text(encoding="utf-8")
    release_site_index = (
        PROJECT_ROOT / "release-site" / "src" / "pages" / "index.astro"
    ).read_text(encoding="utf-8")
    release_site_profile = (
        PROJECT_ROOT / "release-site" / "src" / "pages" / "profiles" / "[id].astro"
    ).read_text(encoding="utf-8")

    assert "channels.json" in release_site_loader
    assert "health.json" not in release_site_loader
    assert "data.health" not in release_site_index
    assert "data.health" not in release_site_profile
