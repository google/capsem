"""Direct behavior tests for release foundation modules."""

from __future__ import annotations

import json
from pathlib import Path

from capsem_builder.release.tools import release_channel_author, release_cohort

REPOSITORY_ROOT = Path(__file__).resolve().parents[3]


def test_release_authoring_and_cohort_use_the_invoking_checkout() -> None:
    assert release_channel_author.PROJECT_ROOT == REPOSITORY_ROOT
    assert release_cohort.PROJECT_ROOT == REPOSITORY_ROOT


def test_unpublished_before_writes_one_empty_verified_cohort(tmp_path: Path) -> None:
    manifest = release_cohort.unpublished_before("nightly", tmp_path)

    assert json.loads(manifest.read_text(encoding="utf-8")) == {
        "channel": "nightly",
        "packages": [],
        "profiles": {},
    }
    report = json.loads(
        (tmp_path / "release-inputs.json").read_text(encoding="utf-8")
    )
    assert report == {
        "allow_empty_profiles": True,
        "artifacts": [],
        "kind": "profiles",
        "manifest_url": manifest.as_uri(),
        "output": str(tmp_path),
        "schema": "capsem.release_inputs.v1",
    }
