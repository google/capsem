"""Impact-aware full-test admission is deterministic and fail-closed."""

from pathlib import Path

import pytest
from capsem_builder.cache.admission import decide_admission
from capsem_builder.cache.models import (
    CachePolicy,
    CacheScope,
    PruneStrategy,
    StagePolicy,
)
from capsem_builder.cache.models import TestAdmissionPolicy as AdmissionPolicy
from capsem_builder.cache.models import TestRoute as Route


def policy() -> CachePolicy:
    return CachePolicy(
        version=1,
        root=Path("cache"),
        authority_environment="CAPSEM_TEST_CACHE_AUTHORITY",
        stages={
            "state": StagePolicy(
                path=Path("state"),
                description="test cache",
                scope=CacheScope.DISK,
                warm_size_bytes=2,
                max_size_bytes=3,
                prune_strategy=PruneStrategy.LRU,
                maximum_age_hours=72,
            )
        },
        test_admission=AdmissionPolicy(
            minimum_commits=10,
            state_path=Path("state/test-admission.jsonl"),
            routes=(
                Route(prefix="config/settings/", groups=("binaries", "release-system")),
                Route(prefix="web/docs/", groups=("release-system",)),
            ),
        ),
    )


def decide(*, failed_attempt: bool = False, **overrides: object):
    values = {
        "policy": policy().test_admission,
        "baseline": "a" * 40,
        "target": "b" * 40,
        "changed_paths": ("config/settings/settings.toml",),
        "commits_since_success": 3,
        "forced": False,
        "force_reason": "",
        "prior_forced": False,
    }
    values.update(overrides)
    return decide_admission(failed_attempt=failed_attempt, **values)


def test_low_impact_repeat_is_refused_with_exact_focus_groups() -> None:
    decision = decide()

    assert decision.allowed is False
    assert decision.groups == ("binaries", "release-system")
    assert "3 of 10 commits" in decision.explanation


def test_low_impact_source_is_allowed_at_the_commit_threshold() -> None:
    decision = decide(commits_since_success=10)

    assert decision.allowed is True
    assert decision.forced is False


def test_unknown_path_fails_closed_to_high_impact_and_allows_full_test() -> None:
    decision = decide(changed_paths=("crates/capsem-core/src/lib.rs",))

    assert decision.allowed is True
    assert decision.high_impact is True


def test_force_requires_a_reason_and_refuses_consecutive_attempts() -> None:
    missing = decide(forced=True)
    repeated = decide(forced=True, force_reason="investigate flake", prior_forced=True)
    first = decide(forced=True, force_reason="investigate flake")

    assert missing.allowed is False
    assert "reason" in missing.explanation
    assert repeated.allowed is False
    assert "consecutive" in repeated.explanation
    assert first.allowed is True
    assert first.forced is True


def test_no_prior_complete_proof_allows_a_baseline_run() -> None:
    decision = decide(baseline=None, changed_paths=())

    assert decision.allowed is True
    assert "baseline" in decision.explanation


@pytest.mark.parametrize("changes", [(), ("config/settings/settings.toml",), ("unknown.rs",)])
@pytest.mark.parametrize("baseline", [None, "a" * 40])
def test_failed_full_attempt_blocks_automatic_retries_regardless_of_impact(changes, baseline) -> None:
    decision = decide(
        failed_attempt=True, baseline=baseline, changed_paths=changes,
        commits_since_success=100,
    )
    assert not decision.allowed
    assert "focused checks" in decision.explanation


def test_approved_retry_remains_possible_after_a_failed_forced_attempt() -> None:
    missing = decide(failed_attempt=True, forced=True, prior_forced=True)
    approved = decide(
        failed_attempt=True, forced=True, prior_forced=True,
        force_reason="User approved retry after repairing the failed owner",
    )
    assert not missing.allowed
    assert approved.allowed and approved.forced
