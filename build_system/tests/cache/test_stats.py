"""The common stats schema reports contract and usage together."""

from pathlib import Path

from capsem_builder.cache.models import (
    CacheInventory,
    CachePolicy,
    CacheScope,
    PruneStrategy,
    StageInventory,
    StagePolicy,
)
from capsem_builder.cache.stats import UsageState, build_stats, render


def test_stats_exposes_every_required_contract_field() -> None:
    policy = CachePolicy(
        version=1,
        root=Path("cache"),
        authority_environment="CAPSEM_TEST_CACHE_AUTHORITY",
        stages={
            "objects": StagePolicy(
                description="immutable build objects",
                scope=CacheScope.DISK,
                path=Path("objects"),
                warm_size_bytes=20,
                max_size_bytes=30,
                prune_strategy=PruneStrategy.LRU,
                maximum_age_hours=1,
            )
        },
    )
    inventory = CacheInventory(
        root=Path("/repo/cache"),
        generated_ns=1,
        logical_bytes=25,
        allocated_bytes=25,
        stages=(
            StageInventory(
                stage_id="objects",
                path=Path("/repo/cache/objects"),
                logical_bytes=25,
                allocated_bytes=25,
                protected_bytes=0,
                entries=(),
            ),
        ),
    )

    report = build_stats(inventory, policy)

    assert report.healthy
    assert report.caches[0].state is UsageState.ABOVE_WARM
    assert report.caches[0].description == "immutable build objects"
    assert report.caches[0].scope is CacheScope.DISK
    assert "warm 20.0 B / max 30.0 B" in render(report)
