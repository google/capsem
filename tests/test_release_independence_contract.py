"""Release graph independence contract gates."""

from __future__ import annotations

import json
from copy import deepcopy
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


def test_independent_version_matrix() -> None:
    graph = _fixture_graph()
    channel = "nightly"
    manifest_version = _current_manifest_version(graph, channel)
    manifest_record = _current_manifest_record(graph, channel)
    manifest = graph["manifests"][channel][manifest_version]
    profile = manifest["profiles"]["co-work"]
    architecture = _architecture(profile, "arm64")

    assert manifest_record["revision"] == manifest["revision"]
    assert manifest_record["revision"] != manifest["packages"][0]["version"]
    assert profile["revision"] != manifest["packages"][0]["version"]
    assert architecture["package_inventory_revision"]
    assert architecture["image_revision"]

    mutations = {
        "manifest_revision": (
            lambda candidate: _set_manifest_revision(candidate, channel, manifest_version, "1.0.3"),
            (
                "channels",
                channel,
                "manifests",
                "0",
                "revision",
            ),
            ("manifests", channel, manifest_version, "revision"),
        ),
        "package_version": (
            lambda candidate: _set_package_version(candidate, channel, manifest_version, "1.5.1-nightly"),
            ("manifests", channel, manifest_version, "packages", "0", "version"),
        ),
        "profile_revision": (
            lambda candidate: _set_profile_revision(
                candidate, channel, manifest_version, "co-work", "2026.07.04.1-nightly"
            ),
            ("manifests", channel, manifest_version, "profiles", "co-work", "revision"),
            ("manifests", channel, manifest_version, "profiles", "co-work", "version"),
        ),
        "package_inventory_revision": (
            lambda candidate: _set_architecture_field(
                candidate,
                channel,
                manifest_version,
                "co-work",
                "arm64",
                "package_inventory_revision",
                "2026.0704.1",
            ),
            (
                "manifests",
                channel,
                manifest_version,
                "profiles",
                "co-work",
                "architectures",
                "0",
                "package_inventory_revision",
            ),
        ),
        "image_revision": (
            lambda candidate: _set_architecture_field(
                candidate,
                channel,
                manifest_version,
                "co-work",
                "arm64",
                "image_revision",
                "2026.0704.2",
            ),
            (
                "manifests",
                channel,
                manifest_version,
                "profiles",
                "co-work",
                "architectures",
                "0",
                "image_revision",
            ),
        ),
    }

    for label, (mutate, *allowed) in mutations.items():
        candidate = deepcopy(graph)
        mutate(candidate)
        changed = _changed_paths(graph, candidate)
        assert changed == set(allowed), label


def _fixture_graph() -> dict[str, Any]:
    return json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))


def _current_manifest_record(graph: dict[str, Any], channel: str) -> dict[str, Any]:
    return next(
        item
        for item in graph["channels"][channel]["manifests"]
        if item["status"] == "current"
    )


def _current_manifest_version(graph: dict[str, Any], channel: str) -> str:
    return _current_manifest_record(graph, channel)["version"]


def _architecture(profile: dict[str, Any], architecture: str) -> dict[str, Any]:
    return next(item for item in profile["architectures"] if item["architecture"] == architecture)


def _set_manifest_revision(
    graph: dict[str, Any],
    channel: str,
    manifest_version: str,
    value: str,
) -> None:
    _current_manifest_record(graph, channel)["revision"] = value
    graph["manifests"][channel][manifest_version]["revision"] = value


def _set_package_version(
    graph: dict[str, Any],
    channel: str,
    manifest_version: str,
    value: str,
) -> None:
    graph["manifests"][channel][manifest_version]["packages"][0]["version"] = value


def _set_profile_revision(
    graph: dict[str, Any],
    channel: str,
    manifest_version: str,
    profile_id: str,
    value: str,
) -> None:
    profile = graph["manifests"][channel][manifest_version]["profiles"][profile_id]
    profile["revision"] = value
    profile["version"] = value


def _set_architecture_field(
    graph: dict[str, Any],
    channel: str,
    manifest_version: str,
    profile_id: str,
    architecture: str,
    field: str,
    value: str,
) -> None:
    profile = graph["manifests"][channel][manifest_version]["profiles"][profile_id]
    _architecture(profile, architecture)[field] = value


def _changed_paths(old: Any, new: Any, prefix: tuple[str, ...] = ()) -> set[tuple[str, ...]]:
    if old == new:
        return set()
    if isinstance(old, dict) and isinstance(new, dict):
        paths: set[tuple[str, ...]] = set()
        for key in sorted(set(old) | set(new)):
            paths.update(_changed_paths(old.get(key), new.get(key), (*prefix, str(key))))
        return paths
    if isinstance(old, list) and isinstance(new, list):
        paths = set()
        for index in range(max(len(old), len(new))):
            left = old[index] if index < len(old) else None
            right = new[index] if index < len(new) else None
            paths.update(_changed_paths(left, right, (*prefix, str(index))))
        return paths
    return {prefix}
