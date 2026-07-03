"""Channel switch release graph guards."""

from __future__ import annotations

import json
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]
UPDATE_RS = PROJECT_ROOT / "crates" / "capsem" / "src" / "update.rs"
GRAPH_FIXTURE = (
    PROJECT_ROOT
    / "tests"
    / "capsem-release"
    / "fixtures"
    / "release-graph-stable-nightly.json"
)


def _release_graph() -> dict:
    return json.loads(GRAPH_FIXTURE.read_text(encoding="utf-8"))


def _current_manifest(graph: dict, channel: str) -> dict:
    records = graph["channels"][channel]["manifests"]
    current = [record for record in records if record["status"] == "current"]
    assert len(current) == 1
    version = current[0]["version"]
    return graph["manifests"][channel][version]


def test_resolver_never_selects_revoked_manifest() -> None:
    source = UPDATE_RS.read_text(encoding="utf-8")

    assert "enum ReleaseManifestStatus" in source
    assert "ReleaseManifestStatus::Revoked" in source
    assert ".filter(|manifest| manifest.status != ReleaseManifestStatus::Revoked)" in source
    assert "channel_manifest_resolution_never_selects_revoked_manifest" in source


def test_old_capsem_selects_compatible_supported_manifest() -> None:
    source = UPDATE_RS.read_text(encoding="utf-8")

    assert "fn select_channel_manifest_url" in source
    assert "fn manifest_is_compatible_with_capsem" in source
    assert "ReleaseManifestStatus::Supported => 1" in source
    assert (
        "channel_manifest_resolution_old_capsem_selects_compatible_supported_manifest"
        in source
    )


def test_stable_and_nightly_caches_coexist() -> None:
    source = UPDATE_RS.read_text(encoding="utf-8")

    assert "fn cache_path_for_source" in source
    assert "fn cache_key_for_source" in source
    assert 'd.join("update-checks")' in source
    assert "stable_and_nightly_update_caches_coexist" in source
    assert "https://release.capsem.org/assets/nightly/manifest.json" in source


def test_switch_stable_to_nightly_and_back() -> None:
    graph = _release_graph()
    source = UPDATE_RS.read_text(encoding="utf-8")

    stable_before = _current_manifest(graph, "stable")
    nightly = _current_manifest(graph, "nightly")
    stable_after = _current_manifest(graph, "stable")

    stable_co_work = stable_before["profiles"]["co-work"]
    nightly_co_work = nightly["profiles"]["co-work"]

    assert stable_before == stable_after
    assert stable_before["version"] == "1.4.0"
    assert nightly["version"] == "1.5.0-nightly.20260702"
    assert stable_co_work["revision"] == "2026.07.02.1-stable"
    assert nightly_co_work["revision"] == "2026.07.02.1-nightly"
    assert stable_co_work["min_capsem_version"] == "1.4.0"
    assert nightly_co_work["min_capsem_version"] == "1.4.0"
    stable_arch = stable_co_work["architectures"][0]
    nightly_arch = nightly_co_work["architectures"][0]
    assert stable_arch["config"][0]["digest"]["sha256"] != nightly_arch["config"][0]["digest"]["sha256"]
    assert stable_arch["images"][0]["digest"]["sha256"] != nightly_arch["images"][0]["digest"]["sha256"]
    assert stable_arch["evidence"][0]["kind"] == "abom"
    assert nightly_arch["evidence"][0]["kind"] == "abom"
    assert stable_arch["evidence"][0]["digest"]["blake3"] != nightly_arch["evidence"][0]["digest"]["blake3"]
    assert stable_before["packages"][0]["name"] == "Capsem-1.4.0.pkg"
    assert nightly["packages"][0]["name"] == "Capsem-1.5.0-nightly.20260702.pkg"
    assert stable_before["packages"][0]["binaries"][0]["version"] == "1.4.0"
    assert nightly["packages"][0]["binaries"][0]["version"] == "1.5.0-nightly.20260702"
    assert "stable_to_nightly_manifest_switch_resolves_nightly_updates" in source


def test_switch_never_selects_revoked_records() -> None:
    graph = _release_graph()
    source = UPDATE_RS.read_text(encoding="utf-8")

    for channel, data in graph["channels"].items():
        records = data["manifests"]
        current = [record for record in records if record["status"] == "current"]
        revoked = [record for record in records if record["status"] == "revoked"]
        assert len(current) == 1, channel
        assert revoked, channel
        assert current[0]["url"] not in {record["url"] for record in revoked}

    assert ".filter(|manifest| manifest.status != ReleaseManifestStatus::Revoked)" in source
    assert "channel_manifest_resolution_never_selects_revoked_manifest" in source
