"""Pure retention planning is stable, bounded, and pin-aware."""

from pathlib import Path

from capsem_builder.cache.models import (
    CacheEntry,
    CacheInventory,
    CachePolicy,
    CacheScope,
    PruneStrategy,
    StageInventory,
    StagePolicy,
)
from capsem_builder.cache.planner import plan_clean, plan_prune


def policy() -> CachePolicy:
    return CachePolicy(
        version=1,
        root=Path("cache"),
        authority_environment="CAPSEM_TEST_CACHE_AUTHORITY",
        stages={
            "objects": StagePolicy(
                path=Path("target/objects"),
                description="test cache",
                scope=CacheScope.DISK,
                warm_size_bytes=20,
                max_size_bytes=30,
                prune_strategy=PruneStrategy.LRU,
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
        logical_bytes=total,
        allocated_bytes=total,
        stages=(
            StageInventory(
                stage_id="objects",
                path=Path("/repo/cache/target/objects"),
                logical_bytes=total,
                allocated_bytes=total,
                protected_bytes=sum(item.logical_bytes for item in entries if item.protected),
                entries=entries,
            ),
        ),
    )


def test_prune_uses_stable_lru_order_from_maximum_to_warm_size() -> None:
    report = inventory(entry("z", 10, 1), entry("a", 10, 1), entry("new", 15, 2))

    plan = plan_prune(report, policy())

    assert [action.key for action in plan.actions] == ["a", "z"]
    assert plan.reclaim_bytes == 20
    assert plan.violations == ()


def test_generational_prune_orders_by_creation_instead_of_recent_use() -> None:
    generational = policy().model_copy(
        update={
            "stages": {
                "objects": policy()
                .stages["objects"]
                .model_copy(update={"prune_strategy": PruneStrategy.GENERATIONAL})
            }
        }
    )
    oldest = entry("oldest", 20, 90).model_copy(update={"created_ns": 1})
    least_used = entry("least-used", 20, 2).model_copy(update={"created_ns": 2})

    plan = plan_prune(inventory(oldest, least_used), generational)

    assert [action.key for action in plan.actions] == ["oldest"]


def test_lru_expiration_is_based_on_last_use() -> None:
    recent = entry("recent", 1, 99).model_copy(update={"created_ns": 1})
    configured = policy().model_copy(
        update={
            "stages": {
                "objects": policy().stages["objects"].model_copy(update={"maximum_age_hours": 1})
            }
        }
    )
    report = inventory(recent).model_copy(update={"generated_ns": 99 + 3_599_000_000_000})

    assert plan_prune(report, configured).actions == ()


def test_protected_entries_are_never_selected_and_report_violations() -> None:
    report = inventory(entry("pinned", 40, 1, protected=True), entry("old", 10, 2))

    plan = plan_prune(report, policy())

    assert [action.key for action in plan.actions] == ["old"]
    assert plan.violations == ("objects remains 40 bytes above max size 30",)


def test_none_policy_reports_pressure_without_deleting_tool_internals() -> None:
    locked = policy().model_copy(
        update={
            "stages": {
                "objects": policy()
                .stages["objects"]
                .model_copy(update={"prune_strategy": PruneStrategy.NONE})
            }
        }
    )

    plan = plan_prune(inventory(entry("fingerprint", 40, 1)), locked)

    assert plan.actions == ()
    assert plan.violations == ("objects uses 40 bytes above max size 30",)


def test_prune_enforces_generation_count_even_below_byte_cap() -> None:
    counted = policy().model_copy(
        update={
            "stages": {
                "objects": policy().stages["objects"].model_copy(update={"maximum_count": 2})
            }
        }
    )

    plan = plan_prune(
        inventory(entry("old", 5, 1), entry("middle", 5, 2), entry("new", 5, 3)),
        counted,
    )

    assert [(action.key, action.reason) for action in plan.actions] == [("old", "over count cap")]


def test_count_ignores_metadata_and_preserves_leased_generations() -> None:
    counted = policy().model_copy(
        update={
            "stages": {
                "objects": policy().stages["objects"].model_copy(update={"maximum_count": 1})
            }
        }
    )
    metadata = entry("DIGEST.md", 1, 0).model_copy(update={"managed": False})
    leased = entry("leased", 5, 1, protected=True)

    plan = plan_prune(inventory(metadata, leased, entry("old", 5, 2)), counted)

    assert [action.key for action in plan.actions] == ["old"]


def test_explicit_clean_preserves_metadata_and_active_leases() -> None:
    metadata = entry("DIGEST.md", 1, 0).model_copy(update={"managed": False})
    leased = entry("leased", 5, 1, protected=True)

    plan = plan_clean(inventory(metadata, leased, entry("generation", 5, 2)), "all")

    assert [action.key for action in plan.actions] == ["generation"]
