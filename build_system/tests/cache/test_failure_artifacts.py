"""Failure evidence is typed, bounded, and rooted in one cache stage."""

import json
from pathlib import Path

import pytest
from capsem_builder.cache.failureartifacts import capture_failure
from capsem_builder.cache.failuremodels import FailureEvidenceManifest
from capsem_builder.cache.paths import CachePaths
from pydantic import ValidationError

from .test_runtime_control import controlled_policy


def test_capture_tails_large_files_and_writes_strict_manifest(tmp_path: Path) -> None:
    policy = controlled_policy()
    paths = CachePaths(repository_root=tmp_path, policy=policy)
    source = paths.root / "target/build.log"
    source.parent.mkdir(parents=True)
    source.write_text("0123456789tail", encoding="utf-8")

    destination = capture_failure(
        paths,
        policy,
        label="red gate",
        run_id="run-1",
        source_commit="a" * 40,
        offline=True,
        now_ns=1_000_000_000,
    )

    manifest = FailureEvidenceManifest.model_validate_json(
        destination.joinpath("manifest.json").read_text(encoding="utf-8")
    )
    assert manifest.label == "red-gate"
    assert manifest.files[0].outcome == "truncated"
    assert destination.joinpath("files/target/build.log").read_text() == "456789tail"
    snapshot = json.loads(destination.joinpath("runtime-snapshot.json").read_text())
    assert snapshot["runtimes"][0]["error"] == "offline inventory requested"


def test_invalid_identity_fails_before_creating_evidence(tmp_path: Path) -> None:
    policy = controlled_policy()
    paths = CachePaths(repository_root=tmp_path, policy=policy)

    with pytest.raises(ValidationError, match="source_commit"):
        capture_failure(
            paths,
            policy,
            label="bad",
            source_commit="not-a-commit",
            offline=True,
            now_ns=1_000_000_000,
        )

    assert not paths.stage("logs").exists()
