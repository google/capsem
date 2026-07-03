"""Profile-lane release graph guards."""

from __future__ import annotations

import json
import subprocess
import sys
from copy import deepcopy
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]
RELEASE_GRAPH = PROJECT_ROOT / "crates" / "capsem-admin" / "src" / "release_graph.rs"
DIFF_SCRIPT = PROJECT_ROOT / "scripts" / "check-release-graph-diff.py"
FIXTURE_GRAPH = (
    PROJECT_ROOT
    / "tests"
    / "capsem-release"
    / "fixtures"
    / "release-graph-stable-nightly.json"
)


def test_profile_json_has_min_capsem_not_current_binary() -> None:
    source = RELEASE_GRAPH.read_text(encoding="utf-8")

    assert "pub struct ProfileDocument" in source
    assert "pub min_capsem_version: Option<String>" in source
    profile_document = source.split("pub struct ProfileDocument", maxsplit=1)[1].split(
        "pub struct SoftwareInventoryRow", maxsplit=1
    )[0]
    assert "current_binary" not in profile_document
    assert "current_assets" not in profile_document
    assert "pub struct SoftwareInventoryRow" in source
    assert "pub struct ProfileConfigRef" in source
    assert "pub struct ProfileArchitectureImages" in source
    assert "pub struct ProfileImageArtifactRef" in source
    assert "profile_json_ownership_rejects_current_binary_and_assets" in source


def test_add_profile_image_version_does_not_deprecate_previous() -> None:
    source = RELEASE_GRAPH.read_text(encoding="utf-8")

    assert "pub struct ProfileVersionHistory" in source
    assert "pub fn append_version" in source
    assert "profile_image_versions_append_without_deprecating_previous" in source
    assert "new profile image version appends" in source


def test_removed_profile_image_is_absent_not_status_removed() -> None:
    source = RELEASE_GRAPH.read_text(encoding="utf-8")

    assert "pub fn diff_profile_image_artifacts" in source
    assert "pub removed: Vec<ProfileImageArtifactKey>" in source
    assert "profile_image_versions_removed_image_is_absent_not_status_removed" in source
    assert "removed is represented by absence, not by a status enum" in source


def test_admin_profile_publish_report_is_lane_scoped() -> None:
    admin_source = (PROJECT_ROOT / "crates" / "capsem-admin" / "src" / "main.rs").read_text(
        encoding="utf-8"
    )

    assert "ProfileReleaseSubcommand" in admin_source
    assert "Publish(ProfileReleaseTargetArgs)" in admin_source
    assert "Deprecate(ProfileReleaseTargetArgs)" in admin_source
    assert "Revoke(ProfileReleaseTargetArgs)" in admin_source
    assert "ProfileReleaseStatusArg" in admin_source
    assert "changed_channels: Vec<String>" in admin_source
    assert "changed_manifests: Vec<String>" in admin_source
    assert "changed_profiles: Vec<String>" in admin_source
    assert "profile_release_commands_publish_report_is_lane_scoped" in admin_source
    assert 'vec!["nightly"]' in admin_source
    assert "publishing nightly co-work must not mutate stable" in admin_source


def test_co_work_nightly_update_does_not_touch_stable_or_binaries(tmp_path: Path) -> None:
    old = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    new = deepcopy(old)
    nightly = new["manifests"]["nightly"]["1.5.0-nightly.20260702"]
    profile = nightly["profiles"]["co-work"]

    new["channels"]["nightly"]["manifests"][0]["digest"]["sha256"] = "f" * 64
    profile["revision"] = "2026.07.02.2-nightly"
    profile["config"][0]["digest"]["sha256"] = "f" * 64
    profile["images"][0]["artifacts"][0]["digest"]["sha256"] = "e" * 64
    profile["images"][0]["evidence"][0]["digest"]["blake3"] = "d" * 64

    summary = tmp_path / "profile-lane-summary.json"
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
        "--summary",
        str(summary),
    )

    assert result.returncode == 0, result.stderr
    assert new["channels"]["stable"] == old["channels"]["stable"]
    assert new["manifests"]["stable"] == old["manifests"]["stable"]
    assert nightly["packages"] == old["manifests"]["nightly"]["1.5.0-nightly.20260702"][
        "packages"
    ]

    report = json.loads(summary.read_text(encoding="utf-8"))
    assert report["accepted"] is True
    assert report["lane"] == "profile"
    assert report["channel"] == "nightly"
    assert report["profile"] == "co-work"
    assert report["violations"] == []
    assert "channels.nightly.manifests.0.digest.sha256" in report["allowed_paths"]
    assert (
        "manifests.nightly.1.5.0-nightly.20260702.profiles.co-work.config.0.digest.sha256"
        in report["allowed_paths"]
    )
    assert (
        "manifests.nightly.1.5.0-nightly.20260702.profiles.co-work.images.0.artifacts.0.digest.sha256"
        in report["allowed_paths"]
    )


def test_profile_lane_rejects_other_profile_change(tmp_path: Path) -> None:
    old = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    old_profile = old["manifests"]["nightly"]["1.5.0-nightly.20260702"]["profiles"][
        "co-work"
    ]
    old["manifests"]["nightly"]["1.5.0-nightly.20260702"]["profiles"]["code"] = deepcopy(
        old_profile
    )
    old["manifests"]["nightly"]["1.5.0-nightly.20260702"]["profiles"]["code"][
        "id"
    ] = "code"
    new = deepcopy(old)
    new["manifests"]["nightly"]["1.5.0-nightly.20260702"]["profiles"]["code"][
        "revision"
    ] = "2026.07.02.2-nightly"

    summary = tmp_path / "profile-lane-summary.json"
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
        "--summary",
        str(summary),
    )

    assert result.returncode == 1
    assert (
        "manifests.nightly.1.5.0-nightly.20260702.profiles.code.revision"
        in result.stderr
    )
    report = json.loads(summary.read_text(encoding="utf-8"))
    assert report["accepted"] is False
    assert report["allowed_paths"] == []
    assert report["violations"] == [
        "manifests.nightly.1.5.0-nightly.20260702.profiles.code.revision"
    ]


def _run_policy(
    tmp_path: Path, old: dict, new: dict, *args: str
) -> subprocess.CompletedProcess[str]:
    old_path = tmp_path / "old.json"
    new_path = tmp_path / "new.json"
    old_path.write_text(json.dumps(old), encoding="utf-8")
    new_path.write_text(json.dumps(new), encoding="utf-8")
    return subprocess.run(
        [sys.executable, str(DIFF_SCRIPT), "--old", str(old_path), "--new", str(new_path), *args],
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
