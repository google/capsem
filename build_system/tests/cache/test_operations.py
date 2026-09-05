"""All cache deletion crosses one contained, journaled mutation boundary."""

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
