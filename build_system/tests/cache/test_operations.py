"""All cache deletion crosses one contained, journaled mutation boundary."""

import fcntl
import json
from pathlib import Path

import pytest
from capsem_builder.cache.models import (
    CachePolicy,
    CacheScope,
    PruneAction,
    PrunePlan,
    PruneStrategy,
    StagePolicy,
)
from capsem_builder.cache.operations import apply_prune
from capsem_builder.cache.paths import CachePaths


def paths(repository: Path, *, stage_path: Path = Path("target/objects"), external=False):
    policy = CachePolicy(
        version=1,
        root=Path("cache"),
        authority_environment="CAPSEM_TEST_CACHE_AUTHORITY",
        stages={
            "objects": StagePolicy(
                path=stage_path,
                external=external,
                description="test objects",
                scope=CacheScope.DISK,
                warm_size_bytes=2,
                max_size_bytes=3,
                prune_strategy=(PruneStrategy.EPHEMERAL if external else PruneStrategy.LRU),
                maximum_age_hours=1,
            )
        },
    )
    return CachePaths(repository_root=repository, policy=policy)


def plan(path: Path) -> PrunePlan:
    return PrunePlan(
        generated_ns=1,
        reclaim_bytes=3,
        actions=(
            PruneAction(
                stage_id="objects",
                key="old",
                path=path,
                logical_bytes=3,
                reason="over soft cap",
            ),
        ),
        violations=(),
    )


def test_apply_removes_only_planned_entries_and_journals_the_reason(tmp_path: Path) -> None:
    cache_paths = paths(tmp_path)
    target = cache_paths.stage("objects") / "old"
    target.mkdir(parents=True)
    (target / "payload").write_bytes(b"abc")

    result = apply_prune(cache_paths, plan(target), reason="operator requested")

    assert not target.exists()
    assert result.removed == (target,)
    journal = cache_paths.root / "state/events/cache.jsonl"
    event = json.loads(journal.read_text(encoding="utf-8"))
    assert event["reason"] == "operator requested"
    assert event["removed"] == [str(target)]


def test_apply_refuses_a_target_outside_the_cache_root(tmp_path: Path) -> None:
    outside = tmp_path / "outside"
    outside.write_bytes(b"keep")
    cache_paths = paths(tmp_path / "repository")

    with pytest.raises(ValueError, match="outside cache stage"):
        apply_prune(cache_paths, plan(outside), reason="bad plan")

    assert outside.read_bytes() == b"keep"


def test_apply_prunes_an_explicit_external_disk_stage(tmp_path: Path) -> None:
    external = tmp_path / "scratch" / "capsem-tests"
    cache_paths = paths(tmp_path / "repository", stage_path=external, external=True)
    target = cache_paths.stage("objects") / "run-123"
    target.mkdir(parents=True)

    result = apply_prune(cache_paths, plan(target), reason="expired scratch")

    assert result.removed == (target,)
    assert not target.exists()


def test_native_build_lock_is_rechecked_and_held_through_pruning(tmp_path: Path, monkeypatch) -> None:
    from capsem_builder.cache import operations
    from capsem_builder.cache.leases import active_path

    original = paths(tmp_path)
    stage = original.policy.stages["objects"].model_copy(update={
        "entry_root": Path("debug/incremental"),
        "mutation_locks": (Path("debug/.cargo-lock"),),
    })
    cache_paths = CachePaths(repository_root=tmp_path, policy=original.policy.model_copy(
        update={"stages": {"objects": stage}},
    ))
    target = cache_paths.stage("objects") / "debug/incremental/old"
    target.mkdir(parents=True)
    lock = cache_paths.stage("objects") / "debug/.cargo-lock"
    lock.touch()
    selected = plan(target)

    with lock.open("rb") as descriptor:
        fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        with pytest.raises(ValueError, match="busy"):
            apply_prune(cache_paths, selected, reason="stale incremental state")
    assert target.exists()

    remove = operations._remove

    def checked_remove(path):
        assert active_path(lock), "Cargo could start during reclamation"
        remove(path)

    monkeypatch.setattr(operations, "_remove", checked_remove)
    apply_prune(cache_paths, selected, reason="stale incremental state")
    assert not target.exists()
    assert not active_path(lock), "cleanup kept Cargo locked"


def test_incremental_retention_preserves_outputs_but_explicit_clean_removes_them(tmp_path: Path) -> None:
    from capsem_builder.cache.inventory import scan_inventory
    from capsem_builder.cache.planner import plan_clean, plan_prune

    original = paths(tmp_path)
    stage = original.policy.stages["objects"].model_copy(update={
        "retention_root": Path("debug/incremental"),
        "mutation_locks": (Path("debug/.cargo-lock"),),
        "warm_size_bytes": 3, "max_size_bytes": 4,
    })
    policy = original.policy.model_copy(update={"stages": {"objects": stage}})
    cache_paths = CachePaths(repository_root=tmp_path, policy=policy)
    root = cache_paths.stage("objects")
    state = root / "debug/incremental/old/state"
    compiled = root / "debug/deps/compiled"
    for file in (state, compiled):
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_bytes(b"abc")
    lock = root / "debug/.cargo-lock"
    lock.touch()
    inode = lock.stat().st_ino

    retention = plan_prune(scan_inventory(cache_paths, policy, retention=True), policy)
    apply_prune(cache_paths, retention, reason="capacity recovery")
    assert not state.exists()
    assert compiled.read_bytes() == b"abc"

    cold = plan_clean(scan_inventory(cache_paths, policy), "objects")
    apply_prune(cache_paths, cold, reason="operator requested cold clean")
    assert not compiled.exists()
    assert lock.stat().st_ino == inode, "unlinking a held lock lets a new producer bypass it"


def test_apply_refuses_a_target_through_a_symlinked_parent(tmp_path: Path) -> None:
    cache_paths = paths(tmp_path)
    cache_paths.stage("objects").mkdir(parents=True)
    outside = tmp_path / "outside"
    outside.mkdir()
    victim = outside / "keep"
    victim.write_text("preserve")
    link = cache_paths.stage("objects") / "link"
    link.symlink_to(outside, target_is_directory=True)

    with pytest.raises(ValueError, match="outside cache stage"):
        apply_prune(cache_paths, plan(link / "keep"), reason="bad nested plan")
    assert victim.read_text() == "preserve"
