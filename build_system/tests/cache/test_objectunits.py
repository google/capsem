"""The object store is retained one generation at a time, like Cargo units.

`objects` used to have no retention at all, and could not have had the
generic kind: its only directory children are `blake3/`, `components/` and
`receipts/`, so least-recently-used would have taken every blob at once and
left every receipt naming bytes that were gone -- a hard build failure, not a
miss. It grew past its cap and failed every prune on the machine instead.
"""

import json
import os
from pathlib import Path

import pytest
from capsem_builder.cache.inventory import scan_inventory, scan_retention_inventory
from capsem_builder.cache.leases import shared_use
from capsem_builder.cache.models import CachePolicy, CacheScope, PruneStrategy, StagePolicy
from capsem_builder.cache.objects import ObjectRef, import_file, object_path, verify
from capsem_builder.cache.operations import apply_prune
from capsem_builder.cache.paths import CachePaths
from capsem_builder.cache.planner import plan_prune

LOCK = Path(".object-store.lock")
DAY_NS = 86_400 * 10**9


def cache(tmp_path: Path, *, max_size: int = 10**9, warm_size: int = 10**9) -> CachePaths:
    stage = StagePolicy(
        path=Path("objects"),
        description="test objects",
        scope=CacheScope.DISK,
        warm_size_bytes=warm_size,
        max_size_bytes=max_size,
        prune_strategy=PruneStrategy.LRU,
        maximum_age_hours=24 * 90,
        object_store=True,
        mutation_locks=(LOCK,),
    )
    return CachePaths(
        repository_root=tmp_path,
        policy=CachePolicy(
            version=1,
            root=Path("cache"),
            authority_environment="CAPSEM_TEST_CACHE_AUTHORITY",
            stages={"objects": stage},
        ),
    )


def blob(paths: CachePaths, tmp_path: Path, name: str, size: int) -> ObjectRef:
    source = tmp_path / "sources" / name
    source.parent.mkdir(parents=True, exist_ok=True)
    source.write_bytes(name.encode() * (size // len(name)))
    return import_file(paths, source)


def receipt(paths: CachePaths, name: str, files: dict[str, ObjectRef], used_ns: int) -> Path:
    path = paths.stage("objects") / "components" / "kernel" / f"{name * 64}"[:64]
    path = path.with_name(path.name + ".json")
    path.parent.mkdir(parents=True, exist_ok=True)
    document = {
        "schema_id": "capsem.component-cache.v1",
        "component": "kernel",
        "input_digest": path.stem,
        "files": {relative: reference.model_dump() for relative, reference in files.items()},
    }
    path.write_text(json.dumps(document), encoding="utf-8")
    os.utime(path, ns=(used_ns, used_ns))
    return path


def age(paths: CachePaths, reference: ObjectRef, used_ns: int) -> None:
    os.utime(object_path(paths, reference), ns=(used_ns, used_ns))


def store_with_three_generations(tmp_path: Path, paths: CachePaths):
    now = 1_000 * DAY_NS
    old_kernel = blob(paths, tmp_path, "old", 4000)
    new_kernel = blob(paths, tmp_path, "new", 4000)
    shared = blob(paths, tmp_path, "shared", 3000)
    orphan = blob(paths, tmp_path, "package", 2000)
    for reference, used in ((old_kernel, now - 5 * DAY_NS), (new_kernel, now),
                            (shared, now), (orphan, now - DAY_NS)):
        age(paths, reference, used)
    old = receipt(paths, "a", {"vmlinuz": old_kernel, "initrd": shared}, now - 5 * DAY_NS)
    new = receipt(paths, "b", {"vmlinuz": new_kernel, "initrd": shared}, now)
    views = paths.stage("objects") / "receipts" / "views" / orphan.digest
    views.mkdir(parents=True)
    (views / "package.deb.json").write_text("{}", encoding="utf-8")
    return now, (old_kernel, new_kernel, shared, orphan), (old, new)


def test_each_receipt_is_a_generation_owning_only_its_unshared_objects(tmp_path: Path) -> None:
    paths = cache(tmp_path)
    now, (old_kernel, new_kernel, shared, orphan), _ = store_with_three_generations(tmp_path, paths)

    stage = scan_inventory(paths, paths.policy, now_ns=now).stages[0]
    entries = {entry.key: entry for entry in stage.entries}

    old = entries[f"components/kernel/{'a' * 64}"]
    assert old.relative_path == Path(f"components/kernel/{'a' * 64}.json"), "receipt goes first"
    assert old.member_paths == (object_path(paths, old_kernel).relative_to(paths.stage("objects")),)
    assert entries[f"blake3/{orphan.digest}"].member_paths == (
        Path("receipts/views") / orphan.digest,
    ), "an unreferenced object carries its write-only view receipts"
    assert all(
        object_path(paths, shared).relative_to(paths.stage("objects")) not in entry.member_paths
        for entry in entries.values()
    ), "an object two receipts name belongs to neither"
    assert stage.logical_bytes >= 4000 + 4000 + 3000 + 2000, "shared bytes are still counted"
    assert new_kernel.digest not in {key.removeprefix("blake3/") for key in entries}


def test_pressure_evicts_the_least_recently_used_generation_and_keeps_the_rest_whole(
    tmp_path: Path,
) -> None:
    paths = cache(tmp_path, max_size=12_000, warm_size=10_000)
    now, (old_kernel, new_kernel, shared, orphan), (old, new) = store_with_three_generations(
        tmp_path, paths
    )

    plan = plan_prune(scan_retention_inventory(paths, paths.policy, now_ns=now), paths.policy)
    apply_prune(paths, plan, reason="recover to warm")

    assert not old.exists() and not object_path(paths, old_kernel).exists()
    assert new.exists()
    for reference in (new_kernel, shared):
        verify(paths, reference)  # the surviving generation is complete
    assert plan.violations == ()
    assert object_path(paths, orphan).exists(), "warm size was reached before the orphan"


def test_an_object_orphaned_by_eviction_is_collected_on_the_next_prune(tmp_path: Path) -> None:
    paths = cache(tmp_path, max_size=1, warm_size=1)
    now, (_, _, shared, _), _ = store_with_three_generations(tmp_path, paths)

    for _ in range(2):
        plan = plan_prune(scan_retention_inventory(paths, paths.policy, now_ns=now), paths.policy)
        apply_prune(paths, plan, reason="recover to warm")

    assert not object_path(paths, shared).exists(), "once no receipt names it, it is collectable"
    assert not list((paths.stage("objects") / "blake3").rglob("*/*")), "every object is gone"


def test_a_generation_in_use_is_not_pruned(tmp_path: Path) -> None:
    paths = cache(tmp_path, max_size=1, warm_size=1)
    now, *_ = store_with_three_generations(tmp_path, paths)

    with shared_use(paths, "objects"):
        plan = plan_prune(scan_retention_inventory(paths, paths.policy, now_ns=now), paths.policy)
    assert plan.actions == (), "a reader holding the store protects all of it"


def test_a_reader_arriving_after_planning_stops_the_removal(tmp_path: Path) -> None:
    paths = cache(tmp_path, max_size=1, warm_size=1)
    now, references, _ = store_with_three_generations(tmp_path, paths)
    plan = plan_prune(scan_retention_inventory(paths, paths.policy, now_ns=now), paths.policy)
    assert plan.actions

    with shared_use(paths, "objects"), pytest.raises(ValueError, match="busy"):
        apply_prune(paths, plan, reason="raced a reader")
    for reference in references:
        verify(paths, reference)


def test_an_object_store_cannot_also_be_a_cargo_stage() -> None:
    with pytest.raises(ValueError, match="object_store"):
        StagePolicy(
            path=Path("objects"),
            description="both",
            scope=CacheScope.DISK,
            warm_size_bytes=1,
            max_size_bytes=2,
            prune_strategy=PruneStrategy.LRU,
            maximum_age_hours=1,
            object_store=True,
            cargo_target_roots=(Path("debug"),),
        )
