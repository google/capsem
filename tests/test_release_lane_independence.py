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
