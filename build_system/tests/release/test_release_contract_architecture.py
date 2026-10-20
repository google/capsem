"""Release graph architecture contract gates."""

from __future__ import annotations

import json
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[3]
FIXTURE_GRAPH = (
    PROJECT_ROOT / "tests" / "capsem-release" / "fixtures" / "release-graph-stable-nightly.json"
)
RELEASE_OUTPUT_DOC = (
    PROJECT_ROOT
    / "web"
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
        assert "catalog" not in current

        manifest = graph["manifests"][channel][current["version"]]
        assert set(manifest) == {"version", "channel", "status", "packages", "runtime"}


def test_no_catalog_or_config_side_channel() -> None:
    """The runtime is images and evidence; nothing else rides along with it."""
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    serialized = json.dumps(graph, sort_keys=True)

    assert "catalog" not in serialized
    assert '"config"' not in serialized

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        assert current["url"] == f"/assets/{channel}/manifest.json"
        runtime = graph["manifests"][channel][current["version"]]["runtime"]
        assert isinstance(runtime, dict)
        assert runtime["architectures"], channel


def test_graph_invariants() -> None:
    doc = RELEASE_OUTPUT_DOC.read_text(encoding="utf-8")

    required = [
        "channels.json -> /assets/<channel>/manifest.json",
        "channel -> packages -> binaries",
        "channel -> runtime -> architectures -> software/images/evidence",
        "There is no `removed` status.",
        "Do not publish HMAC fields in the graph.",
        "The JSON files are the source of truth.",
    ]
    for phrase in required:
        assert phrase in doc


def test_independent_versions() -> None:
    doc = RELEASE_OUTPUT_DOC.read_text(encoding="utf-8")

    required = [
        "Manifest versions, package versions, and runtime revisions are independent.",
        "A package release may change without changing the runtime revision or runtime images.",
        "A runtime revision may change without changing package versions.",
        "The runtime may declare `min_capsem_version`; it must not select the current Capsem binary.",
    ]
    for phrase in required:
        assert phrase in doc


def test_manifest_has_independent_version() -> None:
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel_id, channel in graph["channels"].items():
        current = next(item for item in channel["manifests"] if item["status"] == "current")
        assert current["version"].startswith("1.0.")
        assert graph["manifests"][channel_id][current["version"]]["version"] == current["version"]
        manifest = graph["manifests"][channel_id][current["version"]]
        package_versions = {package["version"] for package in manifest["packages"]}
        assert current["version"] not in package_versions
        assert current["version"] != manifest["runtime"]["revision"]
        assert manifest["runtime"]["revision"] not in package_versions


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
            runtime = manifest["runtime"]
            statuses.append(runtime["status"])
            for architecture in runtime["architectures"]:
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


def test_manifest_history_immutable_auditable() -> None:
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    expected_statuses = {"current", "supported", "deprecated", "revoked"}

    for channel_id, channel in graph["channels"].items():
        records = channel["manifests"]
        assert {record["status"] for record in records} == expected_statuses
        assert len({record["version"] for record in records}) == len(records)
        assert all(record["url"].endswith("/manifest.json") for record in records)

        current = next(record for record in records if record["status"] == "current")
        assert current["url"] == f"/assets/{channel_id}/manifest.json"

        historical = [record for record in records if record["status"] != "current"]
        assert historical
        for record in historical:
            assert record["url"] == (f"/manifests/{channel_id}/{record['version']}/manifest.json")


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


def test_runtime_paths_are_channel_revision_arch_payloads() -> None:
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        runtime = graph["manifests"][channel][current["version"]]["runtime"]
        for architecture in runtime["architectures"]:
            arch = architecture["architecture"]
            prefix = f"/runtime/releases/{channel}/{runtime['revision']}/{arch}/"
            for item in (*architecture["images"], *architecture["evidence"]):
                assert item["url"].startswith(prefix), item["url"]
            for row in architecture["software"]:
                assert row["evidence"].startswith(prefix), row["evidence"]
