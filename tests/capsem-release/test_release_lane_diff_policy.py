"""Release graph diff policy tests."""

from __future__ import annotations

import json
import subprocess
import sys
from copy import deepcopy
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = PROJECT_ROOT / "scripts" / "check-release-graph-diff.py"
FIXTURE_GRAPH = (
    PROJECT_ROOT
    / "tests"
    / "capsem-release"
    / "fixtures"
    / "release-graph-stable-nightly.json"
)


def test_binary_allowed_diff(tmp_path: Path) -> None:
    old = _graph()
    new = deepcopy(old)
    new["channels"]["stable"]["manifests"][0]["digest"]["sha256"] = "c" * 64
    new["manifests"]["stable"]["1.4.0"]["packages"][0]["digest"]["sha256"] = "d" * 64
    new["manifests"]["stable"]["1.4.0"]["binaries"][0]["digest"]["blake3"] = "e" * 64

    result = _run_policy(tmp_path, old, new, "--lane", "binary", "--channel", "stable")

    assert result.returncode == 0, result.stderr


def test_profile_allowed_diff(tmp_path: Path) -> None:
    old = _graph()
    new = deepcopy(old)
    new["channels"]["nightly"]["manifests"][0]["digest"]["sha256"] = "c" * 64
    new["manifests"]["nightly"]["1.5.0-nightly.1"]["profiles"]["co-work"]["images"][0][
        "artifacts"
    ][0]["digest"]["blake3"] = "d" * 64

    result = _run_policy(
        tmp_path,
        old,
        new,
        "--lane",
        "profile",
        "--channel",
        "nightly",
        "--profile",
        "co-work",
    )

    assert result.returncode == 0, result.stderr


def test_cross_channel_change_rejected_without_allowance(tmp_path: Path) -> None:
    old = _graph()
    new = deepcopy(old)
    new["channels"]["stable"]["manifests"][0]["digest"]["sha256"] = "c" * 64
    new["manifests"]["stable"]["1.4.0"]["profiles"]["co-work"]["revision"] = "2026.07.02.2"
    new["manifests"]["nightly"]["1.5.0-nightly.1"]["profiles"]["co-work"]["revision"] = (
        "2026.07.02.2"
    )

    result = _run_policy(
        tmp_path,
        old,
        new,
        "--lane",
        "profile",
        "--channel",
        "nightly",
        "--profile",
        "co-work",
    )

    assert result.returncode == 1
    assert "channels.stable.manifests.0.digest.sha256" in result.stderr
    assert "manifests.stable.1.4.0.profiles.co-work.revision" in result.stderr
    assert "manifests.nightly.1.5.0-nightly.1.profiles.co-work.revision" not in result.stderr


def test_binary_lane_rejects_profile_changes(tmp_path: Path) -> None:
    old = _graph()
    new = deepcopy(old)
    new["manifests"]["stable"]["1.4.0"]["profiles"]["co-work"]["revision"] = "2026.07.02.2"

    result = _run_policy(tmp_path, old, new, "--lane", "binary", "--channel", "stable")

    assert result.returncode == 1
    assert "manifests.stable.1.4.0.profiles.co-work.revision" in result.stderr


def test_fixture_can_mutate_nightly_co_work_only(tmp_path: Path) -> None:
    old = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    new = deepcopy(old)
    new["channels"]["nightly"]["manifests"][0]["digest"]["sha256"] = "f" * 64
    new["manifests"]["nightly"]["1.5.0-nightly.20260702"]["profiles"]["co-work"][
        "images"
    ][0]["artifacts"][0]["digest"]["sha256"] = "e" * 64

    result = _run_policy(
        tmp_path,
        old,
        new,
        "--lane",
        "profile",
        "--channel",
        "nightly",
        "--profile",
        "co-work",
    )

    assert result.returncode == 0, result.stderr
    assert new["channels"]["stable"] == old["channels"]["stable"]
    assert new["manifests"]["stable"] == old["manifests"]["stable"]


def _run_policy(
    tmp_path: Path, old: dict, new: dict, *args: str
) -> subprocess.CompletedProcess[str]:
    old_path = tmp_path / "old.json"
    new_path = tmp_path / "new.json"
    old_path.write_text(json.dumps(old), encoding="utf-8")
    new_path.write_text(json.dumps(new), encoding="utf-8")
    return subprocess.run(
        [sys.executable, str(SCRIPT), "--old", str(old_path), "--new", str(new_path), *args],
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def _graph() -> dict:
    return {
        "channels": {
            "stable": {
                "manifests": [
                    {
                        "version": "1.4.0",
                        "status": "current",
                        "url": "/manifests/stable/1.4.0/manifest.json",
                        "digest": _digest("a"),
                    }
                ]
            },
            "nightly": {
                "manifests": [
                    {
                        "version": "1.5.0-nightly.1",
                        "status": "current",
                        "url": "/manifests/nightly/1.5.0-nightly.1/manifest.json",
                        "digest": _digest("b"),
                    }
                ]
            },
        },
        "manifests": {
            "stable": {"1.4.0": _manifest("stable", "1.4.0")},
            "nightly": {"1.5.0-nightly.1": _manifest("nightly", "1.5.0-nightly.1")},
        },
    }


def _manifest(channel: str, version: str) -> dict:
    return {
        "version": version,
        "status": "current",
        "packages": [{"name": f"capsem-{channel}.pkg", "digest": _digest("a")}],
        "binaries": [{"name": "capsem", "digest": _digest("b")}],
        "profiles": {"co-work": _profile(channel)},
    }


def _profile(channel: str) -> dict:
    return {
        "id": "co-work",
        "revision": f"2026.07.02.1-{channel}",
        "min_capsem_version": "1.4.0",
        "images": [
            {
                "architecture": "arm64",
                "artifacts": [{"kind": "rootfs", "digest": _digest("a")}],
                "evidence": [{"kind": "abom", "digest": _digest("b")}],
            }
        ],
    }


def _digest(seed: str) -> dict:
    return {"sha256": seed * 64, "blake3": seed * 64, "hmac": f"hmac-{seed}"}
