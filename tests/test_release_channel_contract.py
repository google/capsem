"""Release channel machine-contract gates."""

from __future__ import annotations

import json
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[1]
FIXTURE_GRAPH = (
    PROJECT_ROOT
    / "tests"
    / "capsem-release"
    / "fixtures"
    / "release-graph-stable-nightly.json"
)


def test_manifest_version_independent() -> None:
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        package_versions = {package["version"] for package in manifest["packages"]}
        profile_versions = {
            profile["revision"] for profile in manifest["profiles"].values()
        }

        assert current["version"] == manifest["version"], channel
        assert "+assets." not in manifest["version"], channel
        assert manifest["version"] not in package_versions, channel
        assert manifest["version"] not in profile_versions, channel

    admin_source = (PROJECT_ROOT / "crates" / "capsem-admin" / "src" / "main.rs").read_text(
        encoding="utf-8"
    )
    assert "fn validate_graph_manifest_version" in admin_source
    assert 'version.contains("+assets.")' in admin_source
    assert "manifest version must be independent from asset and binary versions" in admin_source
    assert "release_graph_manifest_version_is_independent_from_package_and_assets" in admin_source
