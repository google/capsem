"""Lease control inodes cannot become independently removable generations."""

import os
from pathlib import Path

import pytest
from capsem_builder.cache.inventory import scan_inventory
from capsem_builder.cache.leases import release_path, retain_path
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
from capsem_builder.cache.planner import plan_clean, plan_prune


def fixture(tmp_path, patterns):
    stage = StagePolicy(
        path=Path("images"), description="real generation lease fixture",
        scope=CacheScope.DISK, warm_size_bytes=1, max_size_bytes=2,
        prune_strategy=PruneStrategy.LRU, maximum_age_hours=1,
        maximum_count=1, managed_globs=patterns, lease_template=".{key}.lock",
    )
    policy = CachePolicy(
        version=1, root=Path("cache"), authority_environment="CAPSEM_TEST_CACHE_AUTHORITY",
        stages={"images": stage},
    )
    paths = CachePaths(repository_root=tmp_path, policy=policy)
    root = paths.stage("images")
    for key in ("live", "old"):
        (root / key).mkdir(parents=True)
        (root / key / "payload").write_bytes(key.encode())
    return paths, policy, root


@pytest.mark.parametrize("patterns", [("*",), (".*", "*"), ("[!.]*",)])
@pytest.mark.parametrize("clean", [False, True])
def test_permissive_inventory_cannot_prune_active_lease_inode(tmp_path, patterns, clean):
    paths, policy, root = fixture(tmp_path, patterns)
    lease = root / ".live.lock"
    held = retain_path(lease)
    identity = os.fstat(held.fileno()).st_ino
    try:
        inventory = scan_inventory(paths, policy)
        entries = inventory.stages[0].entries
        assert ".live.lock" not in {entry.key for entry in entries}
        live = next(entry for entry in entries if entry.key == "live")
        assert live.protected and live.member_paths == (Path(".live.lock"),)
        plan = plan_clean(inventory, "images") if clean else plan_prune(inventory, policy)
        result = apply_prune(paths, plan, reason="temporary regression fixture")
        assert root / "old" in result.removed
        assert lease.stat().st_ino == identity
        assert (root / "live/payload").read_bytes() == b"live"
    finally:
        release_path(lease)


@pytest.mark.parametrize("key", [".live.lock", "other-generation"])
def test_stale_lease_as_generation_plan_is_rejected_without_unlinking(tmp_path, key):
    paths, _, root = fixture(tmp_path, ("*",))
    lease = root / ".live.lock"
    held = retain_path(lease)
    try:
        plan = PrunePlan(
            generated_ns=1, reclaim_bytes=0, violations=(),
            actions=(PruneAction(
                stage_id="images", key=key, path=lease,
                logical_bytes=0, reason="old buggy inventory",
            ),),
        )
        with pytest.raises(ValueError, match="lease control"):
            apply_prune(paths, plan, reason="temporary stale-plan regression")
        assert lease.stat().st_ino == os.fstat(held.fileno()).st_ino
        assert (root / "live/payload").read_bytes() == b"live"
    finally:
        release_path(lease)


def test_owner_leasing_after_plan_preserves_generation_and_inode(tmp_path):
    paths, policy, root = fixture(tmp_path, ("*",))
    plan = plan_clean(scan_inventory(paths, policy), "images")
    lease = root / ".live.lock"
    held = retain_path(lease)
    try:
        result = apply_prune(paths, plan, reason="temporary late-owner regression")
        assert root / "live" in result.busy
        assert lease.stat().st_ino == os.fstat(held.fileno()).st_ino
        assert (root / "live/payload").read_bytes() == b"live"
    finally:
        release_path(lease)
