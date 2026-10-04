from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest
from capsem_builder.release.tools import check_runtime_release_delta as DELTA

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "build_system" / "scripts" / "release" / "check-runtime-release-delta.py"
REVISION = "0.7.0-0123456789ab"


def _source(revision: str | None) -> dict[str, object]:
    manifest: dict[str, object] = {"channel": "nightly", "packages": []}
    if revision is not None:
        manifest["runtime"] = {"revision": revision, "architectures": []}
    return manifest


@pytest.mark.parametrize(
    ("source_revision", "needed", "reason"),
    [
        (None, True, "new_runtime"),
        ("0.7.0-ba9876543210", True, "runtime_changed"),
        (REVISION, False, "already_authored"),
    ],
)
def test_runtime_delta_compares_the_one_runtime_revision(
    source_revision: str | None, needed: bool, reason: str
) -> None:
    result = DELTA.runtime_release_delta(_source(source_revision), "nightly", REVISION)

    assert result == {
        "schema": "capsem.runtime_release_delta.v1",
        "channel": "nightly",
        "runtime_revision": REVISION,
        "source_revision": source_revision,
        "release_needed": needed,
        "reason": reason,
    }


def test_runtime_delta_rejects_another_channel_or_malformed_runtime() -> None:
    with pytest.raises(ValueError, match="expected 'stable'"):
        DELTA.runtime_release_delta(_source(None), "stable", REVISION)
    with pytest.raises(ValueError, match="runtime must be an object"):
        DELTA.runtime_release_delta({"channel": "nightly", "runtime": []}, "nightly", REVISION)
    with pytest.raises(ValueError, match="has no revision"):
        DELTA.runtime_release_delta({"channel": "nightly", "runtime": {}}, "nightly", REVISION)


def test_runtime_delta_cli_writes_github_outputs(tmp_path: Path) -> None:
    source = tmp_path / "source.json"
    source.write_text(json.dumps(_source(REVISION)), encoding="utf-8")
    github_output = tmp_path / "github-output"
    report = tmp_path / "delta.json"

    result = subprocess.run(
        [
            sys.executable,
            str(SCRIPT),
            "--source-manifest",
            str(source),
            "--channel",
            "nightly",
            "--runtime-revision",
            REVISION,
            "--json-output",
            str(report),
        ],
        cwd=ROOT,
        env={**os.environ, "GITHUB_OUTPUT": str(github_output)},
        capture_output=True,
        text=True,
        check=False,
    )

    assert result.returncode == 0, result.stderr
    assert github_output.read_text(encoding="utf-8").splitlines() == [
        "release_needed=false",
        "reason=already_authored",
    ]
    assert json.loads(report.read_text(encoding="utf-8"))["reason"] == "already_authored"
