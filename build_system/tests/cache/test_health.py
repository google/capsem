"""Cache health makes every configured capacity limit observable."""

from pathlib import Path

from capsem_builder.cache.health import Pressure, assess
from capsem_builder.cache.models import (
    CacheEntry,
    CacheInventory,
    CachePolicy,
    PruneMethod,
    StageInventory,
    StagePolicy,
)


def report(*, size: int, count: int, free: int = 100) -> tuple[CacheInventory, CachePolicy]:
    configured = StagePolicy(
        path=Path("target/objects"),
        warning_bytes=10,
        soft_bytes=20,
        hard_bytes=30,
        prune=PruneMethod.LRU,
        maximum_age_hours=1,
        maximum_count=2,
    )
    entries = tuple(
        CacheEntry(
            key=str(index),
            relative_path=Path(str(index)),
            logical_bytes=size // count,
            allocated_bytes=size // count,
            created_ns=1,
            last_used_ns=1,
        )
        for index in range(count)
    )
    inventory = CacheInventory(
        root=Path("/repo/cache"),
        generated_ns=1,
        filesystem_free_bytes=free,
        logical_bytes=size,
        allocated_bytes=size,
        stages=(
            StageInventory(
                stage_id="objects",
                path=Path("/repo/cache/target/objects"),
                external=False,
                logical_bytes=size,
                allocated_bytes=size,
                protected_bytes=0,
                entries=entries,
            ),
        ),
    )
    policy = CachePolicy(
        version=1,
        root=Path("cache"),
        minimum_free_bytes=50,
        stages={"objects": configured},
    )
    return inventory, policy


def test_warning_is_visible_without_becoming_a_hard_failure() -> None:
    inventory, policy = report(size=15, count=1)

    health = assess(inventory, policy)

    assert health.healthy
    assert health.stages[0].pressure is Pressure.WARNING


def test_hard_count_and_free_space_violations_are_typed() -> None:
    inventory, policy = report(size=33, count=3, free=40)

    health = assess(inventory, policy)

    assert not health.healthy
    assert health.stages[0].pressure is Pressure.HARD
    assert len(health.violations) == 3
