"""Runtime-lane release graph guards."""

from __future__ import annotations

import json
import subprocess
import sys
from copy import deepcopy
from pathlib import Path

from rust_sources import sibling_tests

PROJECT_ROOT = Path(__file__).resolve().parents[2]
RELEASE_GRAPH = PROJECT_ROOT / "crates" / "capsem-admin" / "src" / "release_graph.rs"
DIFF_SCRIPT = PROJECT_ROOT / "build_system" / "scripts" / "release" / "check-release-graph-diff.py"
FIXTURE_GRAPH = (
    PROJECT_ROOT / "tests" / "capsem-release" / "fixtures" / "release-graph-stable-nightly.json"
)


def test_runtime_json_has_min_capsem_not_current_binary() -> None:
    source = RELEASE_GRAPH.read_text(encoding="utf-8")

    assert "pub struct RuntimeDocument" in source
    assert "pub min_capsem_version: Option<String>" in source
    runtime_document = source.split("pub struct RuntimeDocument", maxsplit=1)[1].split(
        "pub struct SoftwareInventoryRow", maxsplit=1
    )[0]
    assert "current_binary" not in runtime_document
    assert "current_assets" not in runtime_document
    assert "pub struct RuntimeArchitecture" in source
    assert "pub struct RuntimeImageArtifactRef" in source
    assert "runtime_document_validates_and_carries_min_capsem_not_current_binary" in (
        sibling_tests(RELEASE_GRAPH)
    )


def test_admin_runtime_release_is_one_lane_scoped_command() -> None:
    admin_source = (PROJECT_ROOT / "crates" / "capsem-admin" / "src" / "main.rs").read_text(
        encoding="utf-8"
    )

    assert "Validate(ReleaseValidateArgs)" in admin_source
    assert "Release(ReleaseArgs)" in admin_source
    assert "changed_channels: Vec<String>" in admin_source
    assert "changed_manifests: Vec<String>" in admin_source
    assert "changed_image_artifacts: usize" in admin_source
    assert "compatible_with_current_binary: bool" in admin_source
    assert "runtime_revision: String" in admin_source
    assert "publication_identity: String" in admin_source


def test_nightly_runtime_update_does_not_touch_stable_or_binaries(
    tmp_path: Path,
) -> None:
    old = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    new = deepcopy(old)
    nightly = new["manifests"]["nightly"]["1.0.2"]
    runtime = nightly["runtime"]

    new["channels"]["nightly"]["manifests"][0]["digest"]["sha256"] = "f" * 64
    runtime["revision"] = "1.1.1-nightly"
    runtime["architectures"][0]["images"][0]["digest"]["sha256"] = "e" * 64
    runtime["architectures"][0]["evidence"][0]["digest"]["blake3"] = "d" * 64

    summary = tmp_path / "runtime-lane-summary.json"
    result = _run_policy(
        tmp_path,
        old,
        new,
        "--lane",
        "runtime",
        "--channel",
        "nightly",
        "--summary",
        str(summary),
    )

    assert result.returncode == 0, result.stderr
    assert new["channels"]["stable"] == old["channels"]["stable"]
    assert new["manifests"]["stable"] == old["manifests"]["stable"]
    assert nightly["packages"] == old["manifests"]["nightly"]["1.0.2"]["packages"]

    report = json.loads(summary.read_text(encoding="utf-8"))
    assert report["accepted"] is True
    assert report["lane"] == "runtime"
    assert report["channel"] == "nightly"
    assert report["violations"] == []
    assert "channels.nightly.manifests.0.digest.sha256" in report["allowed_paths"]
    assert "manifests.nightly.1.0.2.runtime.revision" in report["allowed_paths"]
    assert (
        "manifests.nightly.1.0.2.runtime.architectures.0.images.0.digest.sha256"
        in report["allowed_paths"]
    )


def test_runtime_lane_rejects_other_channel_runtime_change(tmp_path: Path) -> None:
    old = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    new = deepcopy(old)
    new["manifests"]["stable"]["1.0.2"]["runtime"]["revision"] = "1.1.1-stable"

    summary = tmp_path / "runtime-lane-summary.json"
    result = _run_policy(
        tmp_path,
        old,
        new,
        "--lane",
        "runtime",
        "--channel",
        "nightly",
        "--summary",
        str(summary),
    )

    assert result.returncode == 1
    assert "manifests.stable.1.0.2.runtime.revision" in result.stderr
    report = json.loads(summary.read_text(encoding="utf-8"))
    assert report["accepted"] is False
    assert report["allowed_paths"] == []
    assert report["violations"] == ["manifests.stable.1.0.2.runtime.revision"]


def _run_policy(
    tmp_path: Path, old: dict, new: dict, *args: str
) -> subprocess.CompletedProcess[str]:
    old_path = tmp_path / "old.json"
    new_path = tmp_path / "new.json"
    old_path.write_text(json.dumps(old), encoding="utf-8")
    new_path.write_text(json.dumps(new), encoding="utf-8")
    return subprocess.run(
        [
            sys.executable,
            str(DIFF_SCRIPT),
            "--old",
            str(old_path),
            "--new",
            str(new_path),
            *args,
        ],
        check=False,
        text=True,
        capture_output=True,
    )
