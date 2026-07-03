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


def test_status_enum() -> None:
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
                for item in profile["config"]:
                    collect_status(item)
                images = profile["images"].values() if isinstance(profile["images"], dict) else profile["images"]
                for image in images:
                    for item in image["artifacts"]:
                        collect_status(item)
                    for item in image["evidence"]:
                        collect_status(item)

    assert statuses
    assert set(statuses) <= allowed
    assert "removed" not in statuses
