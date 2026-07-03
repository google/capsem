"""Release hash and evidence contract gates."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import re
import sys
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


def test_reject_placeholder_hashes() -> None:
    checker = _readiness_checker_module()

    for char in "0123f":
        failures = checker.check_release_graph_digest(
            {"sha256": char * 64, "blake3": char * 64},
            f"fixture {char}",
        )

        assert f"fixture {char} digest sha256 uses placeholder pattern" in failures
        assert f"fixture {char} digest blake3 uses placeholder pattern" in failures


def test_repeated_row_digest_theater(monkeypatch) -> None:
    checker = _readiness_checker_module()
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    profile = json.loads(json.dumps(graph["manifests"]["stable"]["1.4.0"]["profiles"]["co-work"]))
    architecture = profile["architectures"][0]
    assert len(architecture["software"]) >= 2

    first = architecture["software"][0]
    second = architecture["software"][1]
    second["digest"] = first["digest"]
    first_image = architecture["images"][0]
    second_image = architecture["images"][1]
    second_image["digest"] = first_image["digest"]

    monkeypatch.setattr(
        checker,
        "fetch_text",
        lambda _url: checker.FetchText(
            text="co-work Co-work 2026.07.02.1-stable arm64"
        ),
    )
    monkeypatch.setattr(
        checker,
        "check_release_graph_artifact",
        lambda *_args, **_kwargs: [],
    )

    failures = checker.check_release_graph_profile(
        "https://release.capsem.test",
        "stable",
        "co-work",
        profile,
    )

    assert (
        f"profile co-work architecture arm64 software digest {first['digest']['sha256']} "
        f"is reused by {first['name']} and {second['name']}"
    ) in failures
    assert (
        f"profile co-work architecture arm64 image digest {first_image['digest']['sha256']} "
        f"is reused by {first_image['url']} and {second_image['url']}"
    ) in failures


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


def _readiness_checker_module():
    module_path = PROJECT_ROOT / "scripts" / "check-remote-release-readiness.py"
    spec = importlib.util.spec_from_file_location("check_remote_release_readiness", module_path)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


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
