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


def test_a_run_is_reclaimed_even_when_its_checkout_is_gone_first(tmp_path: Path) -> None:
    """A checkout can go before its process does -- a pytest basetemp, a
    removed agent worktree -- and the run then stayed behind with `capsem:
    left ... No such file or directory: .../config/cache.toml`. The policy is
    the authority's as much as the checkout's."""
    source = _launch_source(tmp_path)
    authority = tmp_path / "authority"
    (authority / "config").mkdir(parents=True)
    (authority / "config/cache.toml").write_bytes((source / "config/cache.toml").read_bytes())
    probe = f"""
import os, shutil
from pathlib import Path
from capsem_builder import gatelaunch
source = Path({str(source)!r})
environment = gatelaunch.contained_environment(source)
gatelaunch.hold_environment(environment, source)
print(environment["TMPDIR"])
shutil.rmtree(source)
"""
    result = subprocess.run(
        [sys.executable, "-c", probe],
        env={**{k: v for k, v in os.environ.items() if k != AUTHORITY}, AUTHORITY: str(authority)},
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )

    assert result.returncode == 0, result.stderr
    directory = Path(result.stdout.strip().splitlines()[-1])
    assert directory.is_relative_to(tmp_path / "scratch/capsem-tests")
    assert not directory.exists(), result.stderr
    assert result.stderr == ""


# Issue #272: test-temp is namespaced by sha256(authority)[:8], and prune only
# ever looked at its own authority's namespace. A namespace whose authority was
# gone -- a deleted worktree, a test's temporary checkout -- was never looked at
# again: ~350 of them held 2.4 GB on one machine. Liveness is the run lock's,
# whichever namespace it is in, so every namespace is swept.

FOREIGN = "deadbeef"


def foreign(paths: CachePaths, namespace: str = FOREIGN) -> Path:
    directory = paths.stage("test-temp").parent / namespace
    directory.mkdir(parents=True, exist_ok=True)
    return directory


def foreign_run(namespace: Path, key: str) -> Path:
    directory = namespace / key
    (directory / "pytest").mkdir(parents=True)
    (directory / "pytest/payload").write_bytes(b"y" * 64)
    (namespace / f".{key}.lock").touch()
    return directory


def own_live_run(paths: CachePaths) -> tuple[Path, Path]:
    directory = run(paths, "run-3")
    lease = paths.stage("test-temp") / ".run-3.lock"
    leases.retain_path(lease)
    return directory, lease


def test_a_retired_namespace_is_reclaimed_whole_and_the_current_one_is_untouched(
    tmp_path: Path,
) -> None:
    paths = scratch(tmp_path)
    own, lease = own_live_run(paths)
    retired = foreign(paths)
    foreign_run(retired, "run-11")
    (retired / ".run-12.lock").touch()
    empty = foreign(paths, "0123abcd")
    try:
        inventory = scan_inventory(paths, paths.policy)
        keys = {entry.key for entry in inventory.stages[0].entries}
        assert {"run-3", f"{FOREIGN}/run-11", f"{FOREIGN}/run-12"} <= keys
        _, result = prune(paths)
    finally:
        leases.release_path(lease)

    assert not retired.exists() and not empty.exists()
    assert retired in result.removed and empty in result.removed
    assert sorted(path.name for path in paths.stage("test-temp").iterdir()) == [
        ".run-3.lock", "run-3",
    ]
    assert (own / "pytest/payload").exists()


def test_an_empty_current_namespace_is_never_removed(tmp_path: Path) -> None:
    paths = scratch(tmp_path)
    paths.stage("test-temp").mkdir(parents=True)

    plan, _ = prune(paths)

    assert plan.actions == ()
    assert paths.stage("test-temp").is_dir()


def test_a_held_run_lock_keeps_its_run_and_its_namespace(tmp_path: Path) -> None:
    paths = scratch(tmp_path)
    namespace = foreign(paths)
    live = foreign_run(namespace, "run-21")
    dead = foreign_run(namespace, "run-22")

    with (namespace / ".run-21.lock").open("rb") as descriptor:
        fcntl.flock(descriptor, fcntl.LOCK_SH)
        plan, _ = prune(paths)

    assert f"{FOREIGN}/run-21" not in {action.key for action in plan.actions}
    assert (live / "pytest/payload").exists() and (namespace / ".run-21.lock").exists()
    assert not dead.exists() and not (namespace / ".run-22.lock").exists()


def test_a_foreign_owner_that_leases_after_the_plan_keeps_its_run(tmp_path: Path) -> None:
    paths = scratch(tmp_path)
    namespace = foreign(paths)
    directory = foreign_run(namespace, "run-31")
    plan = plan_prune(scan_inventory(paths, paths.policy), paths.policy)
    assert [action.key for action in plan.actions] == [f"{FOREIGN}/run-31"] * 2

    lease = namespace / ".run-31.lock"
    leases.retain_path(lease)
    try:
        result = apply_prune(paths, plan, reason="test")
    finally:
        leases.release_path(lease)

    assert (directory / "pytest/payload").exists() and lease.exists()
    assert directory in result.busy and result.removed == ()


def test_a_symlinked_namespace_is_never_followed(tmp_path: Path) -> None:
    paths = scratch(tmp_path)
    elsewhere = tmp_path / "elsewhere"
    foreign_run(elsewhere, "run-41")
    paths.stage("test-temp").mkdir(parents=True)
    link = paths.stage("test-temp").parent / "cafebabe"
    link.symlink_to(elsewhere, target_is_directory=True)

    plan, _ = prune(paths)

    assert plan.actions == ()
    assert link.is_symlink()
    assert (elsewhere / "run-41/pytest/payload").exists() and (elsewhere / ".run-41.lock").exists()


def test_only_run_and_lease_names_in_namespace_names_are_touched(tmp_path: Path) -> None:
    paths = scratch(tmp_path)
    root = paths.stage("test-temp").parent
    namespace = foreign(paths)
    foreign_run(namespace, "run-51")
    (namespace / "notes.txt").write_text("keep", encoding="utf-8")
    (namespace / "capsem-test-old").mkdir()
    (namespace / "run-52").symlink_to(tmp_path / "outside", target_is_directory=True)
    (tmp_path / "outside").mkdir()
    (tmp_path / "outside/keep").write_bytes(b"keep")
    legacy = root / "capsem-test-0904"
    foreign_run(legacy, "run-53")
    not_hex = root / "DEADBEEF"
    foreign_run(not_hex, "run-54")
    (root / "a1b2c3d4").write_text("a file, not a namespace", encoding="utf-8")

    prune(paths)

    assert not (namespace / "run-51").exists()
    assert (namespace / "notes.txt").exists() and (namespace / "capsem-test-old").is_dir()
    assert not (namespace / "run-52").is_symlink(), "a run link is removed as a link"
    assert (tmp_path / "outside/keep").exists()
    assert (legacy / "run-53/pytest/payload").exists() and (legacy / ".run-53.lock").exists()
    assert (not_hex / "run-54/pytest/payload").exists()
    assert (root / "a1b2c3d4").is_file()


@pytest.mark.parametrize(
    "target",
    [
        "capsem-test-0904/run-1",
        "DEADBEEF/run-1",
        "deadbeef/../outside/run-1",
        "deadbeef/run-1/pytest",
        "cafebabe/run-1",
        "cafebabe",
        "0123abcd/run-1",
        "deadbeef/notes.txt",
        "deadbeef/.run-2.lock",
    ],
)
def test_apply_refuses_a_forged_target_outside_a_real_namespace(
    tmp_path: Path, target: str
) -> None:
    from capsem_builder.cache.models import PruneAction, PrunePlan

    paths = scratch(tmp_path)
    root = paths.stage("test-temp").parent
    for name in ("capsem-test-0904", "DEADBEEF", "outside"):
        foreign_run(root / name, "run-1")
    foreign_run(foreign(paths), "run-1")
    (foreign(paths) / "notes.txt").write_text("keep", encoding="utf-8")
    (foreign(paths) / ".run-2.lock").touch()
    foreign_run(foreign(paths, "0123abcd"), "run-1")
    elsewhere = tmp_path / "elsewhere"
    foreign_run(elsewhere, "run-1")
    (root / "cafebabe").symlink_to(elsewhere, target_is_directory=True)
    before = sorted(str(path) for path in tmp_path.rglob("*"))
    plan = PrunePlan(
        generated_ns=1, reclaim_bytes=0, violations=(),
        actions=(PruneAction(
            stage_id="test-temp", key="deadbeef/run-1", path=root / target,
            logical_bytes=0, reason="forged",
        ),),
    )

    with pytest.raises(ValueError, match="outside cache stage"):
        apply_prune(paths, plan, reason="forged")

    assert sorted(str(path) for path in tmp_path.rglob("*")) == before


def test_a_lease_retakes_a_namespace_a_prune_removed_while_it_was_empty(
    tmp_path: Path, monkeypatch
) -> None:
    """A prune removes an empty namespace between an owner creating it and
    creating its lease; the owner recreates it rather than failing."""
    namespace = tmp_path / FOREIGN
    lease = namespace / ".run-61.lock"
    real_open = os.open
    raced = []

    def racing_open(path, *args, **kwargs):
        if not raced:
            raced.append(True)
            os.rmdir(namespace)
        return real_open(path, *args, **kwargs)

    monkeypatch.setattr(leases.os, "open", racing_open)
    descriptor = leases.retain_path(lease)
    monkeypatch.undo()
    try:
        assert raced and lease.is_file()
        assert os.fstat(descriptor.fileno()).st_ino == lease.stat().st_ino
    finally:
        leases.release_path(lease)
