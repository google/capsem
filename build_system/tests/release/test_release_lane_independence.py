"""Release lane independence gates."""

from __future__ import annotations

import json
import re
import subprocess
import sys
from copy import deepcopy
from pathlib import Path
from typing import Any

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[3]
FIXTURE_GRAPH = (
    PROJECT_ROOT / "tests" / "capsem-release" / "fixtures" / "release-graph-stable-nightly.json"
)
DIFF_POLICY = PROJECT_ROOT / "build_system" / "scripts" / "release" / "check-release-graph-diff.py"


def test_binary_update_does_not_touch_the_runtime(tmp_path: Path) -> None:
    old = _fixture_graph()
    new = deepcopy(old)
    channel = "stable"
    version = _current_manifest_version(new, channel)
    old_runtime = _payload(old["manifests"][channel][version]["runtime"])

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

    assert _payload(new["manifests"][channel][version]["runtime"]) == old_runtime
    assert new["manifests"]["nightly"] == old["manifests"]["nightly"]
    assert new["channels"]["nightly"] == old["channels"]["nightly"]

    result = _run_policy(tmp_path, old, new, "--lane", "binary", "--channel", channel)

    assert result.returncode == 0, result.stderr


@pytest.mark.parametrize(("channel", "other"), [("stable", "nightly"), ("nightly", "stable")])
def test_runtime_update_touches_neither_packages_nor_the_other_channel(
    tmp_path: Path, channel: str, other: str
) -> None:
    old = _fixture_graph()
    new = deepcopy(old)
    version = _current_manifest_version(new, channel)
    old_packages = _payload(old["manifests"][channel][version]["packages"])
    old_other = _payload(old["manifests"][other])

    runtime = new["manifests"][channel][version]["runtime"]
    runtime["revision"] = f"1.0.1-{channel}.20260703"
    for architecture in runtime["architectures"]:
        arch = architecture["architecture"]
        architecture["image_revision"] = runtime["revision"]
        architecture["package_inventory_revision"] = runtime["revision"]
        architecture["images"][0]["digest"] = _digest(f"{channel}-{arch}-kernel-1.0.1")
        architecture["software"][0]["version"] = "3.12.12"
        architecture["software"][0]["digest"] = _digest(f"{channel}-{arch}-python-3.12.12")
        architecture["evidence"][0]["digest"] = _digest(f"{channel}-{arch}-abom-1.0.1")
    new["channels"][channel]["manifests"][0]["digest"] = _digest(
        f"{channel}-manifest-after-runtime-1.0.1"
    )

    assert _payload(new["manifests"][channel][version]["packages"]) == old_packages
    assert _payload(new["manifests"][other]) == old_other
    assert new["channels"][other] == old["channels"][other]

    result = _run_policy(tmp_path, old, new, "--lane", "runtime", "--channel", channel)

    assert result.returncode == 0, result.stderr


def test_runtime_lane_refuses_a_package_change(tmp_path: Path) -> None:
    """The runtime lane may not carry a binary change along with it."""
    old = _fixture_graph()
    new = deepcopy(old)
    channel = "stable"
    version = _current_manifest_version(new, channel)
    new["manifests"][channel][version]["runtime"]["revision"] = "1.0.1-stable.20260703"
    new["manifests"][channel][version]["packages"][0]["version"] = "1.4.1"

    result = _run_policy(tmp_path, old, new, "--lane", "runtime", "--channel", channel)

    assert result.returncode != 0
    assert "packages" in result.stdout + result.stderr


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
    assert stable["runtime"]["revision"] == "1.0.0-stable.20260702"
    assert nightly["runtime"]["revision"] == "1.0.0-nightly.20260702"
    assert stable["packages"] != nightly["packages"]
    assert stable["runtime"] != nightly["runtime"]


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
    runtime_revisions = {
        revision
        for manifest in (stable, nightly)
        for revision in (
            manifest["runtime"]["revision"],
            manifest["runtime"]["architectures"][0]["image_revision"],
            manifest["runtime"]["architectures"][0]["package_inventory_revision"],
        )
    }

    assert package_versions == {"1.4.0", "1.5.0-nightly.20260702"}
    assert runtime_revisions == {"1.0.0-stable.20260702", "1.0.0-nightly.20260702"}
    assert stable_version not in package_versions
    assert nightly_version not in package_versions
    assert stable_version not in runtime_revisions
    assert nightly_version not in runtime_revisions


def test_manifest_history_audit_records() -> None:
    graph = _fixture_graph()
    allowed_statuses = {"current", "supported", "deprecated", "revoked"}

    for channel, record in graph["channels"].items():
        manifests = record["manifests"]
        statuses = [manifest["status"] for manifest in manifests]
        versions = [manifest["version"] for manifest in manifests]

        assert statuses == ["current", "supported", "deprecated", "revoked"], channel
        assert "removed" not in statuses, channel
        assert len(versions) == len(set(versions)), channel

        for manifest in manifests:
            assert manifest["status"] in allowed_statuses, f"{channel}:{manifest}"
            assert manifest["url"], f"{channel}:{manifest['version']}"
            digest = manifest["digest"]
            assert set(digest) == {"sha256", "blake3"}, f"{channel}:{manifest['version']}"
            for value in digest.values():
                assert re.fullmatch(r"[0-9a-f]{64}", value), (
                    f"{channel}:{manifest['version']}:{value}"
                )


def _fixture_graph() -> dict[str, Any]:
    return json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))


def _current_manifest_version(graph: dict[str, Any], channel: str) -> str:
    return next(
        item["version"]
        for item in graph["channels"][channel]["manifests"]
        if item["status"] == "current"
    )


def _payload(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"))


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
        capture_output=True,
    )


def _digest(seed: str) -> dict[str, str]:
    import hashlib

    import blake3

    payload = seed.encode("utf-8")
    return {
        "sha256": hashlib.sha256(payload).hexdigest(),
        "blake3": blake3.blake3(payload).hexdigest(),
    }
