"""Release graph architecture contract gates."""

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
RELEASE_OUTPUT_DOC = (
    PROJECT_ROOT
    / "docs"
    / "src"
    / "content"
    / "docs"
    / "architecture"
    / "release-output.md"
)


def test_canonical_manifest_url() -> None:
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")

        assert current["url"] == f"/assets/{channel}/manifest.json"
        assert not current["url"].startswith(f"/manifests/{channel}/")
        assert "profile_catalog" not in current
        assert "catalog" not in current

        manifest = graph["manifests"][channel][current["version"]]
        assert "profiles" in manifest
        assert "profile_catalog" not in manifest
        assert "catalog" not in manifest


def test_no_profile_catalog_side_channel() -> None:
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    serialized = json.dumps(graph, sort_keys=True)

    assert "profile_catalog" not in serialized
    assert "capsem.profile_catalog" not in serialized
    assert "catalog.json" not in serialized

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        assert current["url"] == f"/assets/{channel}/manifest.json"
        manifest = graph["manifests"][channel][current["version"]]
        assert isinstance(manifest["profiles"], dict)
        assert manifest["profiles"], channel


def test_graph_invariants() -> None:
    doc = RELEASE_OUTPUT_DOC.read_text(encoding="utf-8")

    required = [
        "channels.json -> /assets/<channel>/manifest.json",
        "channel -> packages -> binaries",
        "channel -> profiles -> architecture -> config/software/images",
        "There is no `removed` status.",
        "Do not publish HMAC fields in the graph.",
        "The JSON files are the source of truth.",
    ]
    for phrase in required:
        assert phrase in doc


def test_independent_versions() -> None:
    doc = RELEASE_OUTPUT_DOC.read_text(encoding="utf-8")

    required = [
        "Manifest versions, package versions, profile revisions, and profile image revisions are independent.",
        "A package release may change without changing profile revisions or profile images.",
        "A profile revision may change without changing package versions or other profiles.",
        "A profile image revision may change for one profile and architecture without changing other profiles, other architectures, or packages.",
        "A profile may declare `min_capsem_version`; it must not select the current Capsem binary.",
    ]
    for phrase in required:
        assert phrase in doc


def test_manifest_has_independent_version() -> None:
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel_id, channel in graph["channels"].items():
        current = next(item for item in channel["manifests"] if item["status"] == "current")
        assert current["version"].startswith("1.0.")
        assert graph["manifests"][channel_id][current["version"]]["version"] == current["version"]
        package_versions = {
            package["version"]
            for package in graph["manifests"][channel_id][current["version"]]["packages"]
        }
        profile_revisions = {
            profile["revision"]
            for profile in graph["manifests"][channel_id][current["version"]][
                "profiles"
            ].values()
        }
        assert current["version"] not in package_versions
        assert current["version"] not in profile_revisions


def test_one_status_enum_no_removed() -> None:
    doc = RELEASE_OUTPUT_DOC.read_text(encoding="utf-8")
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    allowed = {"current", "supported", "deprecated", "revoked"}

    assert "All release status fields use the same enum:" in doc
    assert "current | supported | deprecated | revoked" in doc
    assert "There is no `removed` status." in doc

    statuses = []

    def collect_status(item: dict) -> None:
        if "status" in item:
            statuses.append(item["status"])

    for channel in graph["channels"].values():
        statuses.extend(item["status"] for item in channel["manifests"])
    for manifests in graph["manifests"].values():
        for manifest in manifests.values():
            statuses.append(manifest["status"])
            statuses.extend(package["status"] for package in manifest["packages"])
            for package in manifest["packages"]:
                statuses.extend(binary["status"] for binary in package["binaries"])
                for item in package.get("evidence", []):
                    collect_status(item)
            for profile in manifest["profiles"].values():
                for architecture in profile["architectures"]:
                    for item in architecture["config"]:
                        collect_status(item)
                    for item in architecture["images"]:
                        collect_status(item)
                    for item in architecture["evidence"]:
                        collect_status(item)

    assert statuses
    assert set(statuses) <= allowed
    assert "removed" not in statuses
    assert not list(_walk_values(graph, "removed"))
    assert not list(_walk_keys(graph, "payload_status"))
    assert not list(_walk_keys(graph, "deprecated"))
    assert not list(_walk_keys(graph, "deprecated_date"))
    assert "payload_status" not in doc
    assert "`deprecated`: true" not in doc


def _walk_keys(value: object, key: str) -> list[str]:
    matches: list[str] = []

    def visit(item: object, path: str) -> None:
        if isinstance(item, dict):
            for item_key, item_value in item.items():
                next_path = f"{path}.{item_key}" if path else str(item_key)
                if item_key == key:
                    matches.append(next_path)
                visit(item_value, next_path)
        elif isinstance(item, list):
            for index, item_value in enumerate(item):
                visit(item_value, f"{path}[{index}]")

    visit(value, "")
    return matches


def _walk_values(value: object, needle: str) -> list[str]:
    matches: list[str] = []

    def visit(item: object, path: str) -> None:
        if isinstance(item, dict):
            for item_key, item_value in item.items():
                next_path = f"{path}.{item_key}" if path else str(item_key)
                visit(item_value, next_path)
        elif isinstance(item, list):
            for index, item_value in enumerate(item):
                visit(item_value, f"{path}[{index}]")
        elif item == needle:
            matches.append(path)

    visit(value, "")
    return matches


def test_profile_paths_are_channel_profile_arch_payloads() -> None:
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        for profile_id, profile in manifest["profiles"].items():
            for architecture in profile["architectures"]:
                arch = architecture["architecture"]
                expected_profile_prefix = f"/profiles/releases/{profile['revision']}/{profile_id}/{arch}/"
                for item in architecture["config"]:
                    assert item["url"].startswith(expected_profile_prefix), item["url"]
                for item in architecture["images"]:
                    assert item["url"].startswith(
                        ("/assets/releases/", expected_profile_prefix)
                    ), item["url"]
                for item in architecture["evidence"]:
                    assert item["url"].startswith(
                        ("/assets/releases/", expected_profile_prefix)
                    ), item["url"]
