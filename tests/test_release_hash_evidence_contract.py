"""Release hash and evidence contract gates."""

from __future__ import annotations

import hashlib
import json
import re
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


def test_full_machine_digests() -> None:
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    categories: set[str] = set()

    for channel, record in graph["channels"].items():
        for manifest_record in record["manifests"]:
            _assert_full_digest(manifest_record["digest"], f"channels.{channel}.manifests")
            categories.add("manifest_record")

        for manifest in graph["manifests"][channel].values():
            for package in manifest["packages"]:
                _assert_full_digest(package["digest"], f"{channel}.packages.{package['name']}")
                categories.add("package")
                for binary in package["binaries"]:
                    _assert_full_digest(
                        binary["digest"],
                        f"{channel}.packages.{package['name']}.binaries.{binary['name']}",
                    )
                    categories.add("binary")
                for evidence in package.get("evidence", []):
                    _assert_full_digest(
                        evidence["digest"],
                        f"{channel}.packages.{package['name']}.evidence.{evidence['kind']}",
                    )
                    categories.add("package_evidence")

            for profile in manifest["profiles"].values():
                for architecture in profile["architectures"]:
                    label = f"{channel}.profiles.{profile['id']}.{architecture['architecture']}"
                    for config in architecture["config"]:
                        _assert_full_digest(config["digest"], f"{label}.config.{config['kind']}")
                        categories.add("profile_config")
                    for image in architecture["images"]:
                        _assert_full_digest(image["digest"], f"{label}.images.{image['kind']}")
                        categories.add("profile_image")
                    for software in architecture["software"]:
                        _assert_full_digest(
                            software["digest"],
                            f"{label}.software.{software['name']}",
                        )
                        categories.add("software")
                    for evidence in architecture["evidence"]:
                        _assert_full_digest(
                            evidence["digest"],
                            f"{label}.evidence.{evidence['kind']}",
                        )
                        categories.add(f"profile_evidence_{evidence['kind']}")

    assert {
        "manifest_record",
        "package",
        "binary",
        "package_evidence",
        "profile_config",
        "profile_image",
        "software",
        "profile_evidence_abom",
        "profile_evidence_obom",
        "profile_evidence_software_inventory",
    } <= categories


def test_software_inventory_row_digests_are_row_owned() -> None:
    _assert_software_rows_do_not_reuse_inventory_digest()


def test_software_rows_do_not_reuse_inventory_digest() -> None:
    _assert_software_rows_do_not_reuse_inventory_digest()


def _assert_software_rows_do_not_reuse_inventory_digest() -> None:
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


def _assert_full_digest(digest: dict, label: str) -> None:
    assert set(digest) == {"sha256", "blake3"}, label
    for name, value in digest.items():
        assert isinstance(value, str), label
        assert re.fullmatch(r"[0-9a-f]{64}", value), f"{label}:{name}:{value}"
        assert "..." not in value, label


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
