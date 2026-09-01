"""Pure retention planning is stable, bounded, and pin-aware."""

from pathlib import Path

from capsem_builder.cache.models import (
    CacheEntry,
    CacheInventory,
    CachePolicy,
    PruneMethod,
    StageInventory,
    StagePolicy,
)
from capsem_builder.cache.planner import plan_prune


def policy() -> CachePolicy:
    return CachePolicy(
        version=1,
        root=Path("cache"),
        minimum_free_bytes=1,
        stages={
            "objects": StagePolicy(
                path=Path("target/objects"),
                warning_bytes=10,
                soft_bytes=20,
                hard_bytes=30,
                prune=PruneMethod.LRU,
                maximum_age_hours=72,
            )
        },
    )


def entry(key: str, size: int, used: int, *, protected: bool = False) -> CacheEntry:
    return CacheEntry(
        key=key,
        relative_path=Path(key),
        logical_bytes=size,
        allocated_bytes=size,
        created_ns=used,
        last_used_ns=used,
        protected=protected,
    )


def inventory(*entries: CacheEntry) -> CacheInventory:
    total = sum(item.logical_bytes for item in entries)
    return CacheInventory(
        root=Path("/repo/cache"),
        generated_ns=100,
        filesystem_free_bytes=1000,
        logical_bytes=total,
        allocated_bytes=total,
        stages=(
            StageInventory(
                stage_id="objects",
                path=Path("/repo/cache/target/objects"),
                external=False,
                logical_bytes=total,
                allocated_bytes=total,
                protected_bytes=sum(item.logical_bytes for item in entries if item.protected),
                entries=entries,
            ),
        ),
    )


def test_prune_uses_stable_lru_order_until_soft_cap() -> None:
    report = inventory(entry("z", 10, 1), entry("a", 10, 1), entry("new", 15, 2))

    plan = plan_prune(report, policy())

    assert [action.key for action in plan.actions] == ["a", "z"]
    assert plan.reclaim_bytes == 20
    assert plan.violations == ()


def test_protected_entries_are_never_selected_and_report_violations() -> None:
    report = inventory(entry("pinned", 40, 1, protected=True), entry("old", 10, 2))

    plan = plan_prune(report, policy())

    assert [action.key for action in plan.actions] == ["old"]
    assert plan.violations == ("objects remains 40 bytes above soft cap 20",)
