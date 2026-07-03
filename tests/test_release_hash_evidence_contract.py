"""Release hash and evidence contract gates."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
from typing import Any

from blake3 import blake3

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


def test_no_hmac() -> None:
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    doc = RELEASE_OUTPUT_DOC.read_text(encoding="utf-8")

    assert "Do not publish HMAC fields in the graph." in doc
    assert list(_hmac_paths(graph)) == []


def test_software_inventory_row_digests_are_row_owned() -> None:
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        for profile_id, profile in manifest["profiles"].items():
            for architecture in profile["architectures"]:
                evidence_digests = {
                    item["digest"]["sha256"]
                    for item in architecture["evidence"]
                    if item.get("kind") == "software_inventory"
                }
                for software in architecture["software"]:
                    label = f"{channel}:{profile_id}:{architecture['architecture']}:{software['name']}"
                    assert software["digest"] == _software_row_digest(software), label
                    assert software["digest"]["sha256"] not in evidence_digests, label


def _software_row_digest(software: dict) -> dict[str, str]:
    row_core = {
        "name": software["name"],
        "version": software["version"],
        "source": software["source"],
        "architecture": software["architecture"],
        "evidence": software["evidence"],
    }
    payload = json.dumps(row_core, separators=(",", ":")).encode("utf-8")
    return {
        "sha256": hashlib.sha256(payload).hexdigest(),
        "blake3": blake3(payload).hexdigest(),
    }


def _hmac_paths(value: Any, path: str = "$") -> list[str]:
    paths = []
    if isinstance(value, dict):
        for key, nested in value.items():
            nested_path = f"{path}.{key}"
            if "hmac" in key.lower():
                paths.append(nested_path)
            paths.extend(_hmac_paths(nested, nested_path))
    elif isinstance(value, list):
        for index, nested in enumerate(value):
            paths.extend(_hmac_paths(nested, f"{path}[{index}]"))
    return paths
