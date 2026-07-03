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
