"""A leased scratch run lives exactly as long as the process that owns it.

On 2026-09-28 `/var/tmp/capsem-tests/<namespace>` held 42 GB: one ~7 GB
`run-<pid>` per release precheck, each left behind when its gate exited, and
~1,900 `.run-<pid>.lock` files back to early September. Nothing removed a run
when it ended, and `prune test-temp` offered 21 MB because a dead run aged like
any retained generation below a 200 GiB maximum. Only a cold clean got the
disk back.
"""

from __future__ import annotations

import fcntl
import os
import subprocess
import sys
from pathlib import Path

import pytest
from capsem_builder.cache import leases
from capsem_builder.cache.config import load_policy
from capsem_builder.cache.inventory import scan_inventory
from capsem_builder.cache.models import CachePolicy, CacheScope, PruneStrategy, StagePolicy
from capsem_builder.cache.operations import apply_prune, reclaim_generation
from capsem_builder.cache.paths import CachePaths
from capsem_builder.cache.planner import plan_prune

ROOT = Path(__file__).resolve().parents[3]
AUTHORITY = load_policy(ROOT).authority_environment


def scratch(tmp_path: Path) -> CachePaths:
    policy = CachePolicy(
        version=1,
        root=Path("cache"),
        authority_environment="CAPSEM_TEST_CACHE_AUTHORITY",
        stages={
            "test-temp": StagePolicy(
                path=tmp_path / "var/tmp/capsem-tests",
                external=True,
                description="test scratch",
                scope=CacheScope.DISK,
                warm_size_bytes=1 << 30,
                max_size_bytes=1 << 31,
                prune_strategy=PruneStrategy.EPHEMERAL,
                maximum_age_hours=24,
                managed_globs=("run-*",),
                lease_template=".{key}.lock",
            )
        },
    )
    return CachePaths(repository_root=tmp_path / "repository", policy=policy)


def run(paths: CachePaths, key: str) -> Path:
    directory = paths.stage("test-temp") / key
    (directory / "pytest").mkdir(parents=True)
    (directory / "pytest/payload").write_bytes(b"x" * 64)
    return directory


def prune(paths: CachePaths):
    plan = plan_prune(scan_inventory(paths, paths.policy), paths.policy)
    return plan, apply_prune(paths, plan, reason="test")


def test_prune_reclaims_a_dead_run_and_its_lease_but_keeps_a_live_one(tmp_path: Path) -> None:
    paths = scratch(tmp_path)
    stage = paths.stage("test-temp")
    dead = run(paths, "run-1")
    (stage / ".run-1.lock").touch()
    unleased = run(paths, "run-2")
    live = run(paths, "run-3")
    held = stage / ".run-3.lock"
    held.touch()
    orphan = stage / ".run-4.lock"
    orphan.touch()

    with held.open("rb") as descriptor:
        fcntl.flock(descriptor, fcntl.LOCK_SH)
        plan, _ = prune(paths)

    assert not plan.violations
    assert sorted(path.name for path in stage.iterdir()) == [".run-3.lock", "run-3"]
    assert not dead.exists() and not unleased.exists() and not orphan.exists()
    assert (live / "pytest/payload").exists()


def test_apply_skips_a_run_whose_owner_appeared_after_the_plan(tmp_path: Path) -> None:
    """A dead pid can be reused: a new owner that leases between plan and
    apply must keep its directory."""
    paths = scratch(tmp_path)
    directory = run(paths, "run-5")
    plan = plan_prune(scan_inventory(paths, paths.policy), paths.policy)
    assert [action.key for action in plan.actions] == ["run-5"]

    lease = paths.stage("test-temp") / ".run-5.lock"
    leases.retain_path(lease)
    try:
        result = apply_prune(paths, plan, reason="test")
    finally:
        leases.release_path(lease)

    assert directory.exists()
    assert result.removed == ()
    assert result.busy == (directory,)


def test_the_owner_reclaims_its_own_run_and_lease_at_the_end(tmp_path: Path) -> None:
    paths = scratch(tmp_path)
    directory = run(paths, "run-6")
    lease_path = paths.stage("test-temp") / ".run-6.lock"
    leases.retain_path(lease_path)

    result = reclaim_generation(paths, "test-temp", "run-6", reason="run ended")

    assert not directory.exists()
    assert not lease_path.exists()
    assert set(result.removed) == {directory, lease_path}


def test_a_lease_taken_across_a_concurrent_unlink_is_retaken_on_the_live_path(
    tmp_path: Path, monkeypatch
) -> None:
    """A pruner unlinks a lease it holds exclusively. A process that opened the
    old inode first would otherwise hold a lock nobody can see."""
    lease = tmp_path / ".run-7.lock"
    lease.touch()
    real_flock = fcntl.flock
    unlinked = []

    def racing_flock(descriptor, operation):
        if not unlinked:
            unlinked.append(True)
            lease.unlink()
        return real_flock(descriptor, operation)

    monkeypatch.setattr(leases.fcntl, "flock", racing_flock)
    descriptor = leases.retain_path(lease)
    monkeypatch.undo()
    try:
        assert lease.exists()
        assert os.fstat(descriptor.fileno()).st_ino == lease.stat().st_ino
        assert leases.active_path(lease)
    finally:
        leases.release_path(lease)


def _launch_source(tmp_path: Path) -> Path:
    source = tmp_path / "checkout"
    (source / "config").mkdir(parents=True)
    policy = (ROOT / "config/cache.toml").read_text(encoding="utf-8")
    scratch_root = tmp_path / "scratch/capsem-tests"
    policy = policy.replace('path = "/var/tmp/capsem-tests"', f'path = "{scratch_root}"')
    (source / "config/cache.toml").write_text(policy, encoding="utf-8")
    (source / "config/gate.toml").write_bytes((ROOT / "config/gate.toml").read_bytes())
    return source


@pytest.mark.parametrize("exit_code", [0, 3])
def test_a_launched_process_leaves_no_run_directory_or_lease_behind(
    tmp_path: Path, exit_code: int
) -> None:
    source = _launch_source(tmp_path)
    probe = f"""
import os, sys
from pathlib import Path
from capsem_builder import gatelaunch
source = Path({str(source)!r})
environment = gatelaunch.contained_environment(source)
os.environ.update(environment)
gatelaunch.hold_environment(environment, source)
run = Path(environment["TMPDIR"])
assert run.is_dir(), "the run directory exists once its lease is held"
(run / "pytest").mkdir()
(run / "pytest" / "big").write_bytes(b"x" * 4096)
print(run)
sys.exit({exit_code})
"""
    result = subprocess.run(
        [sys.executable, "-c", probe],
        env={k: v for k, v in os.environ.items() if k != AUTHORITY},
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )

    assert result.returncode == exit_code, result.stderr
    directory = Path(result.stdout.strip().splitlines()[-1])
    assert directory.is_relative_to(tmp_path / "scratch/capsem-tests")
    assert not directory.exists(), "the run outlived its process"
    assert not (directory.parent / f".{directory.name}.lock").exists()
    assert result.stderr == ""
