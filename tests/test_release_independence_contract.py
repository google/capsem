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

    assert manifest_record["version"] == manifest["version"]
    assert manifest_record["version"] != manifest["packages"][0]["version"]
    assert profile["revision"] != manifest["packages"][0]["version"]
    assert architecture["package_inventory_revision"]
    assert architecture["image_revision"]

    mutations = {
        "manifest_version": (
            lambda candidate: _set_manifest_version(candidate, channel, manifest_version, "1.0.3"),
            (
                "channels",
                channel,
                "manifests",
                "0",
                "version",
            ),
            ("manifests", channel, manifest_version),
            ("manifests", channel, "1.0.3"),
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


def test_switch_stable_to_nightly_via_manifest_url() -> None:
    graph = _fixture_graph()
    stable_url = "/assets/stable/manifest.json"
    nightly_url = "/assets/nightly/manifest.json"

    stable = _manifest_for_url(graph, stable_url)
    nightly = _manifest_for_url(graph, nightly_url)
    stable_snapshot = json.dumps(stable, sort_keys=True, separators=(",", ":"))

    assert stable["channel"] == "stable"
    assert nightly["channel"] == "nightly"
    assert stable["version"] == nightly["version"]
    assert stable["packages"][0]["version"] == "1.4.0"
    assert nightly["packages"][0]["version"] == "1.5.0-nightly.20260702"
    assert stable["profiles"]["co-work"]["revision"].endswith("-stable")
    assert nightly["profiles"]["co-work"]["revision"].endswith("-nightly")
    assert stable["packages"] != nightly["packages"]
    assert stable["profiles"]["co-work"] != nightly["profiles"]["co-work"]
    assert "profile_catalog" not in json.dumps(nightly, sort_keys=True)

    switched_back = _manifest_for_url(graph, stable_url)
    assert json.dumps(switched_back, sort_keys=True, separators=(",", ":")) == stable_snapshot


def test_cowork_nightly_profile_update_is_isolated() -> None:
    old = _fixture_graph()
    new = deepcopy(old)
    channel = "nightly"
    manifest_version = _current_manifest_version(new, channel)
    profile_id = "co-work"
    manifest = new["manifests"][channel][manifest_version]
    profile = manifest["profiles"][profile_id]
    architecture = _architecture(profile, "arm64")

    profile["revision"] = "2026.07.04.1-nightly"
    profile["version"] = "2026.07.04.1-nightly"
    architecture["image_revision"] = "2026.0704.1"
    architecture["package_inventory_revision"] = "2026.0704.1"
    architecture["software"][0]["version"] = "3.12.12"
    architecture["software"][0]["digest"] = _digest("nightly-co-work-arm64-python-3.12.12")
    architecture["config"][0]["digest"] = _digest("nightly-co-work-arm64-mcp-2026.0704.1")
    architecture["images"][0]["digest"] = _digest("nightly-co-work-arm64-rootfs-2026.0704.1")
    architecture["evidence"][0]["digest"] = _digest("nightly-co-work-arm64-abom-2026.0704.1")

    allowed_prefix = (
        "manifests",
        channel,
        manifest_version,
        "profiles",
        profile_id,
    )
    changed = _changed_paths(old, new)

    assert changed
    assert all(path[: len(allowed_prefix)] == allowed_prefix for path in changed)
    assert new["manifests"]["stable"] == old["manifests"]["stable"]
    assert new["channels"]["stable"] == old["channels"]["stable"]
    assert manifest["packages"] == old["manifests"][channel][manifest_version]["packages"]
    assert (
        manifest["profiles"]["code"]
        == old["manifests"][channel][manifest_version]["profiles"]["code"]
    )
    assert (
        _architecture(manifest["profiles"][profile_id], "x86_64")
        == _architecture(
            old["manifests"][channel][manifest_version]["profiles"][profile_id],
            "x86_64",
        )
    )


def test_nightly_binary_update_without_profile_churn() -> None:
    old = _fixture_graph()
    new = deepcopy(old)
    channel = "nightly"
    manifest_version = _current_manifest_version(new, channel)
    manifest = new["manifests"][channel][manifest_version]
    package = manifest["packages"][0]

    package["version"] = "1.5.0-nightly.20260704"
    package["name"] = "Capsem-1.5.0-nightly.20260704.pkg"
    package["url"] = (
        "https://github.com/google/capsem/releases/download/"
        "v1.5.0-nightly.20260704/Capsem-1.5.0-nightly.20260704.pkg"
    )
    package["bytes"] += 4096
    package["digest"] = _digest("nightly-package-1.5.0-20260704")
    package["evidence"][0]["url"] = (
        "https://github.com/google/capsem/releases/download/"
        "v1.5.0-nightly.20260704/capsem-1-5-0-nightly-20260704-pkg-sbom.spdx.json"
    )
    package["evidence"][0]["digest"] = _digest("nightly-package-1.5.0-20260704-sbom")
    for binary in package["binaries"]:
        binary["version"] = package["version"]
        binary["bytes"] += 17
        binary["digest"] = _digest(f"nightly-binary-{binary['name']}-1.5.0-20260704")

    allowed_prefix = ("manifests", channel, manifest_version, "packages")
    changed = _changed_paths(old, new)

    assert changed
    assert all(path[: len(allowed_prefix)] == allowed_prefix for path in changed)
    assert new["manifests"]["stable"] == old["manifests"]["stable"]
    assert new["channels"]["stable"] == old["channels"]["stable"]
    assert new["channels"]["nightly"] == old["channels"]["nightly"]
    assert manifest["profiles"] == old["manifests"][channel][manifest_version]["profiles"]


def _fixture_graph() -> dict[str, Any]:
    return json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))


def _manifest_for_url(graph: dict[str, Any], manifest_url: str) -> dict[str, Any]:
    matches = [
        (channel_id, record)
        for channel_id, channel in graph["channels"].items()
        for record in channel["manifests"]
        if record["url"] == manifest_url and record["status"] == "current"
    ]
    assert len(matches) == 1, manifest_url
    channel_id, record = matches[0]
    assert manifest_url == f"/assets/{channel_id}/manifest.json"
    manifest = graph["manifests"][channel_id][record["version"]]
    assert manifest["version"] == record["version"]
    assert manifest["channel"] == channel_id
    return manifest


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


def _set_manifest_version(
    graph: dict[str, Any],
    channel: str,
    manifest_version: str,
    value: str,
) -> None:
    _current_manifest_record(graph, channel)["version"] = value
    manifest = graph["manifests"][channel].pop(manifest_version)
    manifest["version"] = value
    graph["manifests"][channel][value] = manifest


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


def _digest(seed: str) -> dict[str, str]:
    import blake3
    import hashlib

    payload = seed.encode("utf-8")
    return {
        "sha256": hashlib.sha256(payload).hexdigest(),
        "blake3": blake3.blake3(payload).hexdigest(),
    }
