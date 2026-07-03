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
