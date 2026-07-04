"""Release lane independence gates."""

from __future__ import annotations

import json
import subprocess
import sys
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
DIFF_POLICY = PROJECT_ROOT / "scripts" / "check-release-graph-diff.py"


def test_binary_update_does_not_touch_profiles(tmp_path: Path) -> None:
    old = _fixture_graph()
    new = deepcopy(old)
    channel = "stable"
    version = _current_manifest_version(new, channel)
    old_profiles = _stable_profile_payloads(old, channel, version)

    package = new["manifests"][channel][version]["packages"][0]
    package["version"] = "1.4.1"
    package["name"] = "Capsem-1.4.1.pkg"
    package["url"] = "/packages/stable/1.4.1/Capsem-1.4.1.pkg"
    package["bytes"] += 17
    package["digest"] = _digest("stable-package-1.4.1")
    package["evidence"][0]["url"] = "/packages/stable/1.4.1/capsem-1-4-1-pkg-sbom.spdx.json"
    package["evidence"][0]["digest"] = _digest("stable-package-1.4.1-sbom")
    package["binaries"][0]["version"] = "1.4.1"
    package["binaries"][0]["bytes"] += 5
    package["binaries"][0]["digest"] = _digest("stable-package-1.4.1-capsem-app")
    new["channels"][channel]["manifests"][0]["digest"] = _digest("stable-manifest-after-1.4.1")

    assert _stable_profile_payloads(new, channel, version) == old_profiles
    assert new["manifests"]["nightly"] == old["manifests"]["nightly"]
    assert new["channels"]["nightly"] == old["channels"]["nightly"]

    result = _run_policy(tmp_path, old, new, "--lane", "binary", "--channel", channel)

    assert result.returncode == 0, result.stderr


def test_profile_update_does_not_touch_packages(tmp_path: Path) -> None:
    old = _fixture_graph()
    new = deepcopy(old)
    channel = "stable"
    profile_id = "co-work"
    version = _current_manifest_version(new, channel)
    old_packages = _stable_package_payloads(old, channel, version)
    old_other_profile = json.dumps(
        old["manifests"][channel][version]["profiles"]["code"],
        sort_keys=True,
        separators=(",", ":"),
    )

    profile = new["manifests"][channel][version]["profiles"][profile_id]
    profile["revision"] = "2026.07.02.2-stable"
    profile["version"] = "2026.07.02.2-stable"
    architecture = profile["architectures"][0]
    architecture["images"][0]["digest"] = _digest("stable-co-work-arm64-rootfs-2026.07.02.2")
    architecture["config"][0]["digest"] = _digest("stable-co-work-arm64-profile-2026.07.02.2")
    architecture["evidence"][0]["digest"] = _digest("stable-co-work-arm64-abom-2026.07.02.2")
    new["channels"][channel]["manifests"][0]["digest"] = _digest(
        "stable-manifest-after-co-work-profile-2026.07.02.2"
    )

    assert _stable_package_payloads(new, channel, version) == old_packages
    assert (
        json.dumps(
            new["manifests"][channel][version]["profiles"]["code"],
            sort_keys=True,
            separators=(",", ":"),
        )
        == old_other_profile
    )
    assert new["manifests"]["nightly"] == old["manifests"]["nightly"]
    assert new["channels"]["nightly"] == old["channels"]["nightly"]

    result = _run_policy(
        tmp_path,
        old,
        new,
        "--lane",
        "profile",
        "--channel",
        channel,
        "--profile",
        profile_id,
    )

    assert result.returncode == 0, result.stderr


def test_cowork_nightly_isolated_update(tmp_path: Path) -> None:
    old = _fixture_graph()
    new = deepcopy(old)
    channel = "nightly"
    profile_id = "co-work"
    version = _current_manifest_version(new, channel)
    old_stable = json.dumps(old["manifests"]["stable"], sort_keys=True, separators=(",", ":"))
    old_packages = _stable_package_payloads(old, channel, version)
    old_code_profile = _profile_payload(old, channel, version, "code")
    old_cowork_x86_64 = _profile_architecture_payload(
        old,
        channel,
        version,
        profile_id,
        "x86_64",
    )

    profile = new["manifests"][channel][version]["profiles"][profile_id]
    profile["revision"] = "1.0.1-nightly.20260703"
    profile["version"] = "1.0.1-nightly.20260703"
    architecture = _profile_architecture(profile, "arm64")
    architecture["images"][0]["digest"] = _digest("nightly-co-work-arm64-rootfs-2026.07.03.1")
    architecture["config"][0]["digest"] = _digest("nightly-co-work-arm64-profile-2026.07.03.1")
    architecture["software"][0]["version"] = "3.12.12"
    architecture["software"][0]["digest"] = _digest("nightly-co-work-arm64-python-3.12.12")
    architecture["evidence"][0]["digest"] = _digest("nightly-co-work-arm64-abom-2026.07.03.1")
    new["channels"][channel]["manifests"][0]["digest"] = _digest(
        "nightly-manifest-after-co-work-arm64-2026.07.03.1"
    )

    assert json.dumps(new["manifests"]["stable"], sort_keys=True, separators=(",", ":")) == old_stable
    assert _stable_package_payloads(new, channel, version) == old_packages
    assert _profile_payload(new, channel, version, "code") == old_code_profile
    assert (
        _profile_architecture_payload(new, channel, version, profile_id, "x86_64")
        == old_cowork_x86_64
    )
    assert new["channels"]["stable"] == old["channels"]["stable"]

    result = _run_policy(
        tmp_path,
        old,
        new,
        "--lane",
        "profile",
        "--channel",
        channel,
        "--profile",
        profile_id,
    )

    assert result.returncode == 0, result.stderr


def test_stable_nightly_switch_keeps_channel_state_independent() -> None:
    graph = _fixture_graph()
    stable_version = _current_manifest_version(graph, "stable")
    nightly_version = _current_manifest_version(graph, "nightly")
    stable = graph["manifests"]["stable"][stable_version]
    nightly = graph["manifests"]["nightly"][nightly_version]

    assert graph["channels"]["stable"]["manifests"][0]["url"] == "/assets/stable/manifest.json"
    assert graph["channels"]["nightly"]["manifests"][0]["url"] == "/assets/nightly/manifest.json"
    assert stable_version == "1.0.2"
    assert nightly_version == "1.0.2"
    assert stable["packages"][0]["version"] == "1.4.0"
    assert nightly["packages"][0]["version"] == "1.5.0-nightly.20260702"
    assert stable["profiles"]["co-work"]["revision"] == "1.0.0-stable.20260702"
    assert nightly["profiles"]["co-work"]["revision"] == "1.0.0-nightly.20260702"
    assert stable["packages"] != nightly["packages"]
    assert stable["profiles"]["co-work"] != nightly["profiles"]["co-work"]


def test_manifest_version_independence() -> None:
    graph = _fixture_graph()
    stable_version = _current_manifest_version(graph, "stable")
    nightly_version = _current_manifest_version(graph, "nightly")
    stable = graph["manifests"]["stable"][stable_version]
    nightly = graph["manifests"]["nightly"][nightly_version]

    assert stable_version == "1.0.2"
    assert nightly_version == "1.0.2"
    assert stable["version"] == stable_version
    assert nightly["version"] == nightly_version

    package_versions = {
        stable["packages"][0]["version"],
        nightly["packages"][0]["version"],
        stable["packages"][0]["binaries"][0]["version"],
        nightly["packages"][0]["binaries"][0]["version"],
    }
    profile_versions = {
        stable["profiles"]["co-work"]["revision"],
        nightly["profiles"]["co-work"]["revision"],
        stable["profiles"]["co-work"]["architectures"][0]["image_revision"],
        nightly["profiles"]["co-work"]["architectures"][0]["image_revision"],
        stable["profiles"]["co-work"]["architectures"][0]["package_inventory_revision"],
        nightly["profiles"]["co-work"]["architectures"][0]["package_inventory_revision"],
    }

    assert package_versions == {"1.4.0", "1.5.0-nightly.20260702"}
    assert profile_versions == {
        "1.0.0-stable.20260702",
        "1.0.0-nightly.20260702",
        "2026.07.02.1",
    }
    assert stable_version not in package_versions
    assert nightly_version not in package_versions
    assert stable_version not in profile_versions
    assert nightly_version not in profile_versions


def _fixture_graph() -> dict[str, Any]:
    return json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))


def _current_manifest_version(graph: dict[str, Any], channel: str) -> str:
    return next(
        item["version"]
        for item in graph["channels"][channel]["manifests"]
        if item["status"] == "current"
    )


def _stable_profile_payloads(
    graph: dict[str, Any],
    channel: str,
    version: str,
) -> dict[str, str]:
    profiles = graph["manifests"][channel][version]["profiles"]
    return {
        profile_id: json.dumps(profile, sort_keys=True, separators=(",", ":"))
        for profile_id, profile in profiles.items()
    }


def _stable_package_payloads(
    graph: dict[str, Any],
    channel: str,
    version: str,
) -> str:
    return json.dumps(
        graph["manifests"][channel][version]["packages"],
        sort_keys=True,
        separators=(",", ":"),
    )


def _profile_payload(
    graph: dict[str, Any],
    channel: str,
    version: str,
    profile_id: str,
) -> str:
    return json.dumps(
        graph["manifests"][channel][version]["profiles"][profile_id],
        sort_keys=True,
        separators=(",", ":"),
    )


def _profile_architecture_payload(
    graph: dict[str, Any],
    channel: str,
    version: str,
    profile_id: str,
    architecture: str,
) -> str:
    profile = graph["manifests"][channel][version]["profiles"][profile_id]
    return json.dumps(
        _profile_architecture(profile, architecture),
        sort_keys=True,
        separators=(",", ":"),
    )


def _profile_architecture(profile: dict[str, Any], architecture: str) -> dict[str, Any]:
    return next(item for item in profile["architectures"] if item["architecture"] == architecture)


def _run_policy(
    tmp_path: Path, old: dict[str, Any], new: dict[str, Any], *args: str
) -> subprocess.CompletedProcess[str]:
    old_path = tmp_path / "old.json"
    new_path = tmp_path / "new.json"
    old_path.write_text(json.dumps(old), encoding="utf-8")
    new_path.write_text(json.dumps(new), encoding="utf-8")
    return subprocess.run(
        [sys.executable, str(DIFF_POLICY), "--old", str(old_path), "--new", str(new_path), *args],
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def _digest(seed: str) -> dict[str, str]:
    import blake3
    import hashlib

    payload = seed.encode("utf-8")
    return {
        "sha256": hashlib.sha256(payload).hexdigest(),
        "blake3": blake3.blake3(payload).hexdigest(),
    }
