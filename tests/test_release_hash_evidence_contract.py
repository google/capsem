"""Release hash and evidence contract gates."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any


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
