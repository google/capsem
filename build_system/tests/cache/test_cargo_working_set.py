"""Cargo retention keeps the working set and reclaims other checkouts' units first.

Workspace units are salted by their checkout's path, so every checkout keeps
its own copy of every workspace crate (61-89 per crate on 2026-09-29). Plain
LRU made the next build of the current head pay for a cold rebuild whenever
other checkouts had compiled more recently, and kept the units of checkouts
that no longer exist until they aged out. The workspace wrapper records each
unit's checkout; retention reclaims a unit whose checkout is gone at once,
and under pressure takes other checkouts' units, then shared ones, and the
current checkout's and the authority's last.
"""

from __future__ import annotations

import time
from pathlib import Path

from capsem_builder.cache.cargounits import OWNER_FILE
from capsem_builder.cache.cli import main
from capsem_builder.cache.enforcement import enforce_repository
from capsem_builder.cache.inventory import scan_retention_inventory
from capsem_builder.cache.leases import CARGO_CACHEDIR_TAG
from capsem_builder.cache.models import CachePolicy
from capsem_builder.cache.paths import CachePaths
from capsem_builder.cache.planner import plan_prune
from click.testing import CliRunner

from .test_cargounits import configured, unit

# Real time: enforcement and the CLI date units by the clock.
NOW = time.time_ns()


def owned(root: Path, name: str, digest: str, hours_ago: int, owner: Path | None) -> Path:
    members = unit(root, "debug", name, digest, used_hours_ago=hours_ago, now_ns=NOW)
    fingerprint = members[0].parent
    if owner is not None:
        (fingerprint / OWNER_FILE).symlink_to(owner, target_is_directory=True)
    return fingerprint


def layout(tmp_path: Path):
    """Five units and a policy whose bounds each test then sets from their sizes."""
    policy = configured(max_size=1, warm_size=1)
    authority = tmp_path / "authority"
    current = tmp_path / "worktrees/current"
    other = tmp_path / "worktrees/other"
    for checkout in (authority, current, other):
        checkout.mkdir(parents=True)
    paths = CachePaths(repository_root=authority, policy=policy)
    root = paths.stage("cargo")
    units = {
        # The working set is the least recently used: other checkouts compiled
        # since, which is exactly when plain LRU evicted it.
        "current": owned(root, "capsem-core", "1" * 16, 40, current),
        "main": owned(root, "capsem-proto", "2" * 16, 30, authority),
        "shared": owned(root, "serde", "3" * 16, 20, None),
        "other": owned(root, "capsem-core", "4" * 16, 2, other),
        "gone": owned(root, "capsem-core", "5" * 16, 1, tmp_path / "worktrees/removed"),
    }
    # What enforcement itself keeps in the stage, so it is counted up front.
    for tagged in (root, root / "llvm-cov-target"):
        tagged.mkdir(parents=True, exist_ok=True)
        (tagged / "CACHEDIR.TAG").write_text(CARGO_CACHEDIR_TAG, encoding="utf-8")
    [stage] = scan_retention_inventory(paths, policy, now_ns=NOW).stages
    sizes = {entry.key: entry.logical_bytes for entry in stage.entries}
    size = {name: sizes[f"debug/{path.name.rsplit('-', 1)[1]}"] for name, path in units.items()}
    size["unit-less"] = stage.logical_bytes - sum(size.values())
    return paths, current, units, size


def bounded(paths: CachePaths, *, max_size: int, warm_size: int) -> tuple[CachePaths, CachePolicy]:
    policy = configured(max_size=max_size, warm_size=warm_size)
    return CachePaths(repository_root=paths.repository_root, policy=policy), policy


def removed(plan) -> set[Path]:
    return {action.path for action in plan.actions}


def test_pressure_takes_other_checkouts_then_shared_units_and_keeps_the_working_set(
    tmp_path: Path,
) -> None:
    paths, current, units, size = layout(tmp_path)
    total = sum(size.values())
    paths, policy = bounded(
        paths, max_size=total - 1, warm_size=total - size["gone"] - size["other"]
    )

    plan = plan_prune(scan_retention_inventory(paths, policy, now_ns=NOW, checkout=current), policy)

    assert units["gone"] in removed(plan) and units["other"] in removed(plan)
    assert units["current"] not in removed(plan), "the current head's next build would be cold"
    assert units["main"] not in removed(plan), "the authority's next build would be cold"
    assert units["shared"] not in removed(plan), "shared units went before another checkout's"
    assert not plan.violations


def test_the_working_set_goes_last_and_only_to_reach_warm(tmp_path: Path) -> None:
    paths, current, units, size = layout(tmp_path)
    paths, policy = bounded(
        paths, max_size=sum(size.values()) - 1, warm_size=size["main"] + size["unit-less"]
    )

    plan = plan_prune(scan_retention_inventory(paths, policy, now_ns=NOW, checkout=current), policy)

    assert {units["gone"], units["other"], units["shared"], units["current"]} <= removed(plan)
    assert units["main"] not in removed(plan)


def test_a_unit_whose_checkout_is_gone_is_reclaimed_below_the_maximum(tmp_path: Path) -> None:
    paths, current, units, size = layout(tmp_path)
    paths, policy = bounded(paths, max_size=10 * sum(size.values()), warm_size=sum(size.values()))

    plan = plan_prune(scan_retention_inventory(paths, policy, now_ns=NOW, checkout=current), policy)

    assert removed(plan) == {
        action.path for action in plan.actions if action.key == f"debug/{'5' * 16}"
    }
    assert units["gone"] in removed(plan)
    assert {action.reason for action in plan.actions} == {"owner checkout is gone"}


def test_enforcement_keeps_the_calling_checkouts_units(tmp_path: Path) -> None:
    """End to end: enforcing over the maximum never evicts what the next build
    of the calling checkout's head needs."""
    paths, current, units, size = layout(tmp_path)
    total = sum(size.values())
    paths, policy = bounded(
        paths, max_size=total - 1, warm_size=total - size["gone"] - size["other"]
    )

    result = enforce_repository(paths, policy, "cargo", reason="test", checkout=current)

    assert result.pruned and not result.violations
    assert units["current"].is_dir() and units["main"].is_dir()
    assert not units["other"].exists() and not units["gone"].exists()


def test_the_cache_cli_enforces_on_behalf_of_its_policy_checkout(tmp_path: Path) -> None:
    """`CargoCacheBound` runs `capsem-cache --repository <authority>
    --policy-repository <checkout> enforce cargo`: that checkout is the one
    whose next build must stay warm."""
    paths, current, units, size = layout(tmp_path)
    total = sum(size.values())
    (current / "config").mkdir()
    (current / "config/cache.toml").write_text(
        f"""
version = 1
root = "cache"
authority_environment = "CAPSEM_TEST_CACHE_AUTHORITY"
[stages.cargo]
description = "cargo units"
scope = "disk"
path = "target/cargo"
warm_size_bytes = {total - size["gone"] - size["other"]}
max_size_bytes = {total - 1}
prune_strategy = "lru"
maximum_age_hours = 720
cargo_target_roots = ["debug"]
""".strip(),
        encoding="utf-8",
    )

    result = CliRunner().invoke(
        main,
        [
            "--repository", str(paths.repository_root),
            "--policy-repository", str(current),
            "enforce", "cargo", "--reason", "test",
        ],
    )

    assert result.exit_code == 0, result.output
    assert units["current"].is_dir() and units["main"].is_dir() and units["shared"].is_dir()
    assert not units["other"].exists()
