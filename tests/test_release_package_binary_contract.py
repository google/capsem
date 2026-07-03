"""Release package and executable inventory contract gates."""

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


def test_package_owns_binaries() -> None:
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        packages = manifest["packages"]

        assert packages, channel
        for package in packages:
            assert "name" in package, package
            assert "url" in package, package
            assert "digest" in package, package
            assert package["binaries"], package["name"]
            for binary in package["binaries"]:
                assert "package" not in binary, binary
                assert binary["name"], binary
                assert binary["version"], binary
                assert binary["installed_path"].startswith("/"), binary
                assert len(binary["digest"]["sha256"]) == 64, binary
                assert len(binary["digest"]["blake3"]) == 64, binary
                assert binary["sbom_component_ref"].startswith("SPDXRef-"), binary


def test_sbom_not_repeated_per_binary() -> None:
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        for package in manifest["packages"]:
            package_sboms = [
                item
                for item in package.get("evidence", [])
                if "sbom" in item["kind"].lower()
            ]
            assert package_sboms, package["name"]
            for binary in package["binaries"]:
                assert "evidence" not in binary, binary
                assert "package_evidence" not in binary, binary
                assert "sbom" not in binary, binary
                assert "sbom_component_ref" in binary, binary
