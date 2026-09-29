"""Every cache maximum together, plus headroom, fits a share of the disk.

On 2026-09-29 the configured maxima summed to 839 GiB on a 484 GB build box:
Cargo 180, test-temp 200, Docker 96, Tart 64, objects 48 and a long tail. Each
contract was satisfiable alone and none together, so every cache could stay
under its own maximum while the disk filled to 100% four times in two days and
killed gate and release runs with ENOSPC.
"""

from __future__ import annotations

from pathlib import Path

import pytest
from capsem_builder.cache import budget
from capsem_builder.cache.budget import BudgetError, BudgetPolicy
from capsem_builder.cache.config import load_policy
from capsem_builder.cache.models import CachePolicy, CacheScope, PruneStrategy, StagePolicy

ROOT = Path(__file__).resolve().parents[3]
GIB = 1024**3


def _policy(*maxima: int, fraction: float = 0.5, headroom: int = 10 * GIB, minimum: int = 100 * GIB) -> CachePolicy:
    stages = {
        f"stage-{index}": StagePolicy(
            path=Path(f"target/stage-{index}"),
            description="test cache",
            scope=CacheScope.DISK,
            warm_size_bytes=maximum // 2,
            max_size_bytes=maximum,
            prune_strategy=PruneStrategy.LRU,
            maximum_age_hours=1,
        )
        for index, maximum in enumerate(maxima)
    }
    return CachePolicy(
        version=1,
        root=Path("cache"),
        authority_environment="CAPSEM_TEST_CACHE_AUTHORITY",
        stages=stages,
        budget=BudgetPolicy(
            filesystem_fraction=fraction,
            headroom_bytes=headroom,
            minimum_filesystem_bytes=minimum,
            ephemeral_environment={"RUNNER_ENVIRONMENT": "github-hosted"},
        ),
    )


def test_the_checked_in_maxima_fit_the_declared_machine() -> None:
    policy = load_policy(ROOT)
    assert policy.budget is not None, "the cache policy must declare its disk budget"
    total = budget.total_maxima(policy)
    limit = policy.budget.filesystem_fraction * policy.budget.minimum_filesystem_bytes
    assert total + policy.budget.headroom_bytes <= limit


def test_the_checked_in_cargo_maximum_is_tens_of_gib() -> None:
    """With line tables only and no incremental sessions a full gate run adds
    single-digit GiB; 180 GiB kept a month of dead checkouts instead."""
    assert load_policy(ROOT).stages["cargo"].max_size_bytes <= 64 * GIB


def test_maxima_beyond_the_budget_are_refused_at_load() -> None:
    _policy(20 * GIB, 20 * GIB)  # 40 + 10 <= 0.5 * 100
    with pytest.raises(ValueError, match=r"cache maxima sum to 41\.0 GiB"):
        _policy(20 * GIB, 21 * GIB)


def test_runtimes_count_toward_the_budget() -> None:
    policy = load_policy(ROOT)
    stages = sum(stage.max_size_bytes for stage in policy.stages.values())
    runtimes = sum(runtime.max_size_bytes for runtime in policy.runtimes.values())
    assert runtimes > 0
    assert budget.total_maxima(policy) == stages + runtimes


def test_a_small_filesystem_refuses_a_retained_cache_machine() -> None:
    policy = _policy(20 * GIB)
    budget.verify_machine(policy, filesystem_bytes=60 * GIB, environment={})  # 30 <= 30
    with pytest.raises(BudgetError) as refused:
        budget.verify_machine(policy, filesystem_bytes=59 * GIB, environment={})
    message = str(refused.value)
    assert "20.0 GiB of cache maxima + 10.0 GiB headroom" in message
    assert "stage-0 20.0 GiB" in message, "the largest maxima are named"


def test_an_ephemeral_runner_is_not_a_retained_cache_machine() -> None:
    """A GitHub-hosted runner's caches die with it; its disk is its image's."""
    policy = _policy(20 * GIB)
    budget.verify_machine(
        policy, filesystem_bytes=1 * GIB, environment={"RUNNER_ENVIRONMENT": "github-hosted"}
    )
    with pytest.raises(BudgetError):
        budget.verify_machine(
            policy, filesystem_bytes=1 * GIB, environment={"RUNNER_ENVIRONMENT": "self-hosted"}
        )


def test_a_policy_without_a_budget_is_refused_at_the_gate() -> None:
    unbounded = _policy(1 * GIB).model_copy(update={"budget": None})
    with pytest.raises(BudgetError, match="declares no disk budget"):
        budget.verify_machine(unbounded, filesystem_bytes=10**15, environment={})


@pytest.mark.parametrize("fraction", [0.0, -0.1, 1.01])
def test_the_fraction_is_a_share_of_the_disk(fraction: float) -> None:
    with pytest.raises(ValueError):
        BudgetPolicy(
            filesystem_fraction=fraction,
            headroom_bytes=GIB,
            minimum_filesystem_bytes=GIB,
            ephemeral_environment={},
        )


def test_filesystem_bytes_reads_the_whole_filesystem(tmp_path: Path) -> None:
    assert budget.filesystem_bytes(tmp_path) > 0


def test_an_absent_optional_runtime_holds_nothing_on_this_machine() -> None:
    """Tart is macOS-only and optional: its maximum is real on a Mac and zero
    on the Linux build box, where counting it would demand 64 GiB of nothing."""
    policy = load_policy(ROOT)
    tart = policy.runtimes["tart"]
    assert not tart.required
    everything = budget.total_maxima(policy)
    without = budget.total_maxima(policy, present=lambda command: command != tart.command)
    assert everything - without == tart.max_size_bytes
    docker = policy.runtimes["docker"]
    assert budget.total_maxima(policy, present=lambda _command: False) == (
        everything - tart.max_size_bytes - (0 if docker.required else docker.max_size_bytes)
    )


def test_this_build_box_class_passes_with_margin() -> None:
    """The 484 GB Linux box that filled: 450.8 GiB, Tart absent."""
    policy = load_policy(ROOT)
    box = 484_000_000_000
    budget.verify_machine(
        policy, filesystem_bytes=box, environment={}, present=lambda command: command != "tart"
    )
