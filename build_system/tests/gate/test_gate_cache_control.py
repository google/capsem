"""The gate is a thin caller of the typed cache control plane."""

from pathlib import Path

from capsem_builder.cache.config import load_policy
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.cachecontrol import CacheControl
from capsem_builder.gate.sourcecommit import SourceCommit
from helpers.gate import RecordingRunner

PROJECT_ROOT = Path(__file__).resolve().parents[3]
CONFIG = gate_config.load(PROJECT_ROOT)
CACHE_POLICY = load_policy(PROJECT_ROOT)


def test_host_builder_is_a_typed_retained_cache_owner() -> None:
    policy = CacheControl(RecordingRunner(PROJECT_ROOT)).image_policy("capsem-host-builder")

    assert policy.repository == "capsem-host-builder"
    assert policy.prune_strategy.value == "generational"
    assert policy.maximum_count == 1


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


def test_enforcement_and_reclaim_use_explicit_cache_commands() -> None:
    runner = RecordingRunner(PROJECT_ROOT)
    cache = CacheControl(runner)

    cache.enforce("docker", "candidate")
    cache.reclaim("capsem-install-test", keep="capsem-install-test:current")

    assert runner.matching(r"enforce docker --reason 'gate cache enforcement for candidate'")
    assert runner.matching(
        r"reclaim-image capsem-install-test --keep capsem-install-test:current .*--apply"
    )


def test_receipt_policy_comes_from_validated_cache_policy() -> None:
    policy = CacheControl(RecordingRunner(PROJECT_ROOT)).image_policy("capsem-install-test")

    assert policy.maximum_count == 3
    assert policy.maximum_age_seconds == 336 * 3600
    assert policy.warm_size_bytes == 6 * 1024**3
    assert policy.max_size_bytes == 12 * 1024**3


def test_private_gate_controls_outer_cache_with_private_policy(monkeypatch, tmp_path: Path) -> None:
    prefix = tmp_path / "prefix"
    prefix.mkdir()
    (prefix / "config").symlink_to(PROJECT_ROOT / "config", target_is_directory=True)
    monkeypatch.setenv(CACHE_POLICY.authority_environment, str(PROJECT_ROOT))
    runner = RecordingRunner(prefix)

    CacheControl(runner).enforce("docker", "candidate")

    assert runner.matching(
        rf"capsem-cache --repository {PROJECT_ROOT} --policy-repository {prefix} "
        r"enforce docker"
    )
