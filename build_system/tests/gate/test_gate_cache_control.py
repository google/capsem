"""The gate is a thin caller of the typed cache control plane."""

from pathlib import Path

import pytest
from capsem_builder.gate.cachecontrol import CacheControl
from capsem_builder.gate.errors import GateError
from capsem_builder.gate.sourcecommit import SourceCommit
from helpers.gate import RecordingRunner

PROJECT_ROOT = Path(__file__).resolve().parents[3]


def test_release_names_one_configured_final_consumer_boundary() -> None:
    runner = RecordingRunner(PROJECT_ROOT)

    CacheControl(runner).release("after-install")

    assert runner.matching(
        r"capsem-cache --repository .* release after-install --apply "
        r"--reason 'gate completed cache boundary after-install'"
    )


def test_unknown_release_boundary_fails_before_a_process_runs() -> None:
    runner = RecordingRunner(PROJECT_ROOT)

    with pytest.raises(GateError, match="unknown cache release boundary 'imaginary'"):
        CacheControl(runner).release("imaginary")

    assert runner.commands == []


def test_failure_capture_is_best_effort_and_carries_identity() -> None:
    runner = RecordingRunner(PROJECT_ROOT, failures=["capture-failure"])

    CacheControl(runner).capture_failure(
        label="abcdef123456",
        run_id="20260813-010203-abcdef-release-binaries",
        source_commit=SourceCommit("1" * 40),
    )

    assert runner.matching(
        r"capture-failure --label abcdef123456 "
        r"--run-id 20260813-010203-abcdef-release-binaries --source-commit 1{40}"
    )


def test_capacity_preflight_and_reclaim_use_explicit_cache_commands() -> None:
    runner = RecordingRunner(PROJECT_ROOT)
    cache = CacheControl(runner)

    cache.ensure_space("default", "candidate")
    cache.reclaim("capsem-install-test", keep="capsem-install-test:current")

    assert runner.matching(r"ensure-space default --reason 'gate preflight for candidate'")
    assert runner.matching(
        r"reclaim-image capsem-install-test --keep capsem-install-test:current .*--apply"
    )


def test_receipt_limits_come_from_validated_cache_policy() -> None:
    limits = CacheControl(RecordingRunner(PROJECT_ROOT)).image_limits("capsem-install-test")

    assert limits.maximum_count == 3
    assert limits.maximum_age_seconds == 336 * 3600
    assert limits.maximum_bytes == 96 * 1024**3
