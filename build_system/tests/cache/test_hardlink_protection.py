"""Immutable cache files stay accounted and retained while sessions link them."""

import os
from pathlib import Path

import pytest
from capsem_builder.cache.config import load_policy
from capsem_builder.cache.inventory import scan_inventory
from capsem_builder.cache.models import CachePolicy, CacheScope, PruneStrategy, StagePolicy
from capsem_builder.cache.operations import apply_prune
from capsem_builder.cache.paths import CachePaths
from capsem_builder.cache.planner import plan_clean, plan_prune


def test_oci_cache_protects_payloads_held_by_live_consumers():
    policy = load_policy(Path(__file__).resolve().parents[3])
    assert policy.stages["oci-images"].protect_hardlinks


def cache(tmp_path, *, protect=True):
    options = {"protect_hardlinks": True} if protect else {}
    policy = CachePolicy(
        version=1, root=Path("cache"), authority_environment="CAPSEM_TEST_CACHE_AUTHORITY",
        stages={"payloads": StagePolicy(
            path=Path("payloads"), description="Immutable payloads", scope=CacheScope.DISK,
            warm_size_bytes=1, max_size_bytes=2, maximum_age_hours=1,
            prune_strategy=PruneStrategy.LRU, mutation_locks=(Path("cache.lock"),),
            entry_root=Path("blobs"), **options,
        )},
    )
    paths = CachePaths(repository_root=tmp_path, policy=policy)
    payload = paths.stage("payloads") / "blobs/root"
    payload.parent.mkdir(parents=True)
    payload.write_bytes(b"root filesystem")
    payload.chmod(0o444)
    return paths, payload


def test_active_session_links_are_protected_but_still_count_against_capacity(tmp_path):
    paths, payload = cache(tmp_path)
    first, second = tmp_path / "first", tmp_path / "second"
    os.link(payload, first)
    os.link(payload, second)
    report = scan_inventory(paths, paths.policy)
    stage = report.stages[0]
    assert stage.entries[0].protected
    assert stage.logical_bytes == stage.protected_bytes == len(b"root filesystem")
    selected = plan_prune(report, paths.policy)
    assert not selected.actions
    assert selected.violations, "active payloads cannot silently waive the cache ceiling"
    assert not plan_clean(report, "payloads").actions
    first.unlink()
    assert scan_inventory(paths, paths.policy).stages[0].entries[0].protected
    second.unlink()
    released = scan_inventory(paths, paths.policy)
    assert not released.stages[0].entries[0].protected
    result = apply_prune(paths, plan_prune(released, paths.policy), reason="holder released")
    assert result.removed == (payload,)
    assert not payload.exists()


def test_a_stale_prune_plan_rechecks_links_acquired_after_planning(tmp_path):
    paths, payload = cache(tmp_path)
    selected = plan_prune(scan_inventory(paths, paths.policy), paths.policy)
    assert tuple(action.path for action in selected.actions) == (payload,)
    held = tmp_path / "active-session"
    os.link(payload, held)
    result = apply_prune(paths, selected, reason="stale plan")
    assert result.busy == (payload,)
    assert not result.removed
    assert payload.stat().st_ino == held.stat().st_ino
    assert held.read_bytes() == b"root filesystem"
    held.unlink()
    assert apply_prune(paths, selected, reason="last holder released").removed == (payload,)


def test_link_protection_is_opt_in(tmp_path):
    paths, payload = cache(tmp_path, protect=False)
    held = tmp_path / "link"
    os.link(payload, held)
    selected = plan_prune(scan_inventory(paths, paths.policy), paths.policy)
    assert not selected.violations
    assert apply_prune(paths, selected, reason="ordinary stage").removed == (payload,)
    assert held.read_bytes() == b"root filesystem"


def test_a_hardlinked_symlink_does_not_pin_the_entry_or_follow_its_target(tmp_path):
    paths, payload = cache(tmp_path)
    payload.unlink()
    outside = tmp_path / "outside"
    outside.write_bytes(b"foreign bytes")
    payload.symlink_to(outside)
    held = tmp_path / "linked-symlink"
    os.link(payload, held, follow_symlinks=False)
    report = scan_inventory(paths, paths.policy)
    assert not report.stages[0].entries[0].protected
    result = apply_prune(paths, plan_clean(report, "payloads"), reason="remove owned link")
    assert result.removed == (payload,)
    assert outside.read_bytes() == b"foreign bytes"
    assert held.is_symlink()


def test_link_protection_setting_is_strict(tmp_path):
    paths, _ = cache(tmp_path)
    document = paths.policy.model_dump(mode="json")
    document["stages"]["payloads"]["protect_hardlinks"] = "yes"
    with pytest.raises(ValueError, match="protect_hardlinks"):
        CachePolicy.model_validate(document)
