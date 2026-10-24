"""Projected producer intent preserves actual inventory and configured retention."""

from pathlib import Path

import pytest
from capsem_builder.cache.enforcement import enforce_repository
from capsem_builder.cache.leases import release_path, retain_generation
from capsem_builder.cache.models import CachePolicy, CacheScope, PruneStrategy, StagePolicy
from capsem_builder.cache.paths import CachePaths


def fixture(tmp_path):
    policy = CachePolicy(
        version=1,
        root=Path("cache"),
        authority_environment="CAPSEM_TEST_CACHE_AUTHORITY",
        stages={
            "objects": StagePolicy(
                path=Path("objects"),
                description="synthetic projected admission",
                scope=CacheScope.DISK,
                warm_size_bytes=4,
                max_size_bytes=8,
                maximum_count=4,
                maximum_age_hours=336,
                prune_strategy=PruneStrategy.GENERATIONAL,
                managed_globs=("[!.]*",),
                lease_template=".{key}.lock",
            )
        },
    )
    paths = CachePaths(repository_root=tmp_path, policy=policy)
    for name in ("a", "b", "c", "d"):
        directory = paths.stage("objects") / name
        directory.mkdir(parents=True)
        (directory / "payload").write_bytes(b"xx")
    return paths


def test_equality_needs_explicit_incoming_intent_without_falsifying_actual_bytes(tmp_path):
    from capsem_builder.cache.capacity import RequiredCapacity

    paths = fixture(tmp_path)
    ordinary = enforce_repository(paths, paths.policy, "objects", reason="ordinary exact equality")
    assert (
        ordinary.action_count == 0 and ordinary.before_size_bytes == ordinary.after_size_bytes == 8
    )
    original = paths.policy.model_dump()
    result = enforce_repository(
        paths,
        paths.policy,
        "objects",
        reason="producer needs actual headroom",
        required=RequiredCapacity(bytes=3, entries=1),
    )
    assert not result.violations and result.before_size_bytes == 8
    actual = sum(path.stat().st_size for path in paths.stage("objects").glob("*/payload"))
    assert result.after_size_bytes == actual and actual + 3 <= 8
    assert result.reclaim_bytes == 8 - actual
    assert paths.policy.model_dump() == original


def test_foreign_real_lease_inode_and_payload_survive_projected_pressure(tmp_path):
    from capsem_builder.cache.capacity import RequiredCapacity

    paths = fixture(tmp_path)
    held = retain_generation(paths, "objects", "a")
    lock = paths.stage("objects") / ".a.lock"
    inode = lock.stat().st_ino
    try:
        result = enforce_repository(
            paths,
            paths.policy,
            "objects",
            reason="bounded next producer",
            required=RequiredCapacity(bytes=3, entries=1),
        )
        assert not result.violations and result.after_size_bytes == 2
        assert (paths.stage("objects") / "a/payload").read_bytes() == b"xx"
        assert lock.stat().st_ino == inode and not held.closed
    finally:
        release_path(lock)


def test_all_protected_state_refuses_projection_without_removing_any_generation(tmp_path):
    from capsem_builder.cache.capacity import RequiredCapacity

    paths = fixture(tmp_path)
    for name in ("a", "b", "c", "d"):
        retain_generation(paths, "objects", name)
    try:
        result = enforce_repository(
            paths,
            paths.policy,
            "objects",
            reason="cannot fit another generation",
            required=RequiredCapacity(bytes=1, entries=1),
        )
        assert result.violations and result.action_count == 0
        assert result.before_size_bytes == result.after_size_bytes == 8
        assert all(
            (paths.stage("objects") / name / "payload").read_bytes() == b"xx"
            for name in ("a", "b", "c", "d")
        )
    finally:
        for name in ("a", "b", "c", "d"):
            release_path(paths.stage("objects") / f".{name}.lock")


@pytest.mark.parametrize(
    "values", [{"bytes": -1}, {"entries": -1}, {"bytes": True}, {"entries": True}]
)
def test_capacity_intent_is_strict_and_nonnegative(values):
    from capsem_builder.cache.capacity import RequiredCapacity

    with pytest.raises(ValueError):
        RequiredCapacity.model_validate(values)


def test_impossible_or_ambiguous_intent_fails_before_mutation(tmp_path):
    from capsem_builder.cache.capacity import RequiredCapacity

    paths = fixture(tmp_path)
    for stage, required in (
        ("objects", RequiredCapacity(bytes=9)),
        ("objects", RequiredCapacity(entries=5)),
        ("all", RequiredCapacity(bytes=1)),
    ):
        with pytest.raises(ValueError):
            enforce_repository(
                paths, paths.policy, stage, reason="invalid request", required=required
            )
        assert len(list(paths.stage("objects").glob("*/payload"))) == 4


def test_count_only_projection_reclaims_exactly_one_eligible_generation(tmp_path):
    from capsem_builder.cache.capacity import RequiredCapacity

    paths = fixture(tmp_path)
    result = enforce_repository(
        paths,
        paths.policy,
        "objects",
        reason="one more owned generation",
        required=RequiredCapacity(entries=1),
    )
    assert not result.violations and result.reclaim_bytes == 2
    assert result.before_size_bytes == 8 and result.after_size_bytes == 6
    assert len(list(paths.stage("objects").glob("*/payload"))) == 3
