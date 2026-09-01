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
from capsem_builder.cache.planner import plan_clean, plan_prune


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


def inventory(*entries: CacheEntry, free: int = 1000) -> CacheInventory:
    total = sum(item.logical_bytes for item in entries)
    return CacheInventory(
        root=Path("/repo/cache"),
        generated_ns=100,
        filesystem_free_bytes=free,
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


def test_none_policy_reports_pressure_without_deleting_tool_internals() -> None:
    locked = policy().model_copy(
        update={
            "stages": {
                "objects": policy().stages["objects"].model_copy(update={"prune": PruneMethod.NONE})
            }
        }
    )

    plan = plan_prune(inventory(entry("fingerprint", 40, 1)), locked)

    assert plan.actions == ()
    assert plan.violations == ("objects remains 40 bytes above soft cap 20",)


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


def test_prune_recovers_global_free_space_from_oldest_allocated_entries() -> None:
    reserved = policy().model_copy(update={"minimum_free_bytes": 15})

    plan = plan_prune(
        inventory(entry("old", 10, 1), entry("new", 10, 2), free=0),
        reserved,
    )

    assert [action.key for action in plan.actions] == ["old", "new"]
    assert {action.reason for action in plan.actions} == {"below free-space reserve"}
    assert plan.violations == ()


def test_prune_reports_reserve_when_only_nonprunable_bytes_remain() -> None:
    locked = policy().model_copy(
        update={
            "minimum_free_bytes": 15,
            "stages": {
                "objects": policy().stages["objects"].model_copy(update={"prune": PruneMethod.NONE})
            },
        }
    )

    plan = plan_prune(inventory(entry("fingerprint", 10, 1), free=0), locked)

    assert plan.actions == ()
    assert plan.violations == ("filesystem remains at 0 bytes below free-space reserve 15",)


def test_explicit_clean_preserves_metadata_and_active_leases() -> None:
    metadata = entry("DIGEST.md", 1, 0).model_copy(update={"managed": False})
    leased = entry("leased", 5, 1, protected=True)

    plan = plan_clean(inventory(metadata, leased, entry("generation", 5, 2)), "all")

    assert [action.key for action in plan.actions] == ["generation"]
