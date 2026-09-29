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
    assert plan.violations == (
        "objects remains 40 bytes above max size 30: "
        "40 bytes are protected (no lock is held)",
    )


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


def _leased_scratch() -> CachePolicy:
    """The test-temp shape: ephemeral runs, each alive only while its lease is held."""
    stage = policy().stages["objects"].model_copy(update={
        "prune_strategy": PruneStrategy.EPHEMERAL,
        "warm_size_bytes": 1000,
        "max_size_bytes": 2000,
        "maximum_age_hours": 24,
        "maximum_count": 1,
        "managed_globs": ("run-*",),
        "lease_template": ".{key}.lock",
    })
    return policy().model_copy(update={"stages": {"objects": stage}})


def test_ephemeral_prune_reclaims_a_run_nobody_leases_whatever_its_age_or_size() -> None:
    """A finished run's 7 GiB directory is garbage the moment its owner exits.

    Before, a leased ephemeral stage aged like any generation: a dead run sat
    for `maximum_age_hours` below a 200 GiB maximum, and one run per release
    precheck filled a 484 GB disk before prune saw anything to reclaim.
    """
    dead = entry("run-1", 7, 99).model_copy(
        update={"member_paths": (Path(".run-1.lock"),)}
    )
    live = entry("run-2", 7, 99, protected=True)

    plan = plan_prune(inventory(dead, live), _leased_scratch())

    assert [(action.path.name, action.reason) for action in plan.actions] == [
        ("run-1", "no live owner"),
        (".run-1.lock", "no live owner"),
    ]
    assert plan.reclaim_bytes == 7


def test_orphaned_leases_are_collected_but_never_counted_as_generations() -> None:
    orphan = entry("run-3", 0, 99).model_copy(
        update={"relative_path": Path(".run-3.lock"), "lease_only": True}
    )
    held = entry("run-4", 0, 99, protected=True).model_copy(
        update={"relative_path": Path(".run-4.lock"), "lease_only": True}
    )
    live = entry("run-2", 7, 99, protected=True)

    plan = plan_prune(inventory(orphan, held, live), _leased_scratch())

    assert [(action.path.name, action.reason) for action in plan.actions] == [
        (".run-3.lock", "orphaned lease"),
    ]
    assert plan.violations == (), "a lease file is not a generation for the count cap"


def test_explicit_clean_removes_a_generation_with_its_members() -> None:
    generation = entry("run-1", 5, 1).model_copy(update={"member_paths": (Path(".run-1.lock"),)})

    plan = plan_clean(inventory(generation), "all")

    assert [action.path.name for action in plan.actions] == ["run-1", ".run-1.lock"]
    assert plan.reclaim_bytes == 5


def _sparse(key: str, logical: int, allocated: int, used: int, *, protected: bool = False) -> CacheEntry:
    return CacheEntry(
        key=key,
        relative_path=Path(key),
        logical_bytes=logical,
        allocated_bytes=allocated,
        created_ns=used,
        last_used_ns=used,
        protected=protected,
    )


def _sparse_inventory(*entries: CacheEntry) -> CacheInventory:
    logical = sum(item.logical_bytes for item in entries)
    allocated = sum(item.allocated_bytes for item in entries)
    return CacheInventory(
        root=Path("/repo/cache"),
        generated_ns=100,
        logical_bytes=logical,
        allocated_bytes=allocated,
        stages=(
            StageInventory(
                stage_id="objects",
                path=Path("/repo/cache/target/objects"),
                logical_bytes=logical,
                allocated_bytes=allocated,
                protected_bytes=sum(item.budget_bytes for item in entries if item.protected),
                entries=entries,
            ),
        ),
    )


def test_a_sparse_file_is_budgeted_by_what_it_occupies_on_disk() -> None:
    """A VM's sparse rootfs overlay claims gigabytes and occupies megabytes.

    The release proof for 0.6.4 stopped 97 minutes in on "test-temp remains
    137677756611 bytes above max size 34359738368" while /var/tmp held 2.4 GB:
    the budget summed logical sizes. A disk budget protects the disk, so it
    counts allocated blocks.
    """
    report = _sparse_inventory(_sparse("vm", 1000, 5, 1, protected=True), _sparse("old", 10, 10, 2))

    plan = plan_prune(report, policy())

    assert plan.violations == ()
    assert plan.actions == ()


def test_pressure_is_measured_in_allocated_bytes() -> None:
    # 35 allocated bytes against a 30-byte max: evicting the older entry reaches
    # the 20-byte warm size. Measured logically (2000) both would go.
    report = _sparse_inventory(_sparse("a", 1000, 18, 1), _sparse("b", 1000, 17, 2))

    plan = plan_prune(report, policy())

    assert [action.key for action in plan.actions] == ["a"]


def test_block_rounding_never_counts_more_than_the_content() -> None:
    report = _sparse_inventory(_sparse("a", 1, 20, 1), _sparse("b", 1, 15, 2))

    plan = plan_prune(report, policy())

    assert plan.actions == ()
    assert plan.violations == ()
