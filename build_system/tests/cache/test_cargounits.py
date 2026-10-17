"""Cargo target directories are retained one compilation unit at a time.

Every gate prefix salts workspace units with its checkout path, so each
source state leaves a full set of units behind. Retention that only knew
`debug/incremental` found nothing to reclaim while the stage grew past its
maximum and enforcement refused every run (issue #205).
"""

import fcntl
import os
from pathlib import Path

import pytest
from capsem_builder.cache.cargounits import unaccounted_size
from capsem_builder.cache.inventory import scan_retention_inventory
from capsem_builder.cache.models import CachePolicy, CacheScope, PruneStrategy, StagePolicy
from capsem_builder.cache.operations import apply_prune
from capsem_builder.cache.paths import CachePaths
from capsem_builder.cache.planner import plan_prune

OLD = "0123456789abcdef"
NEW = "fedcba9876543210"
HOUR_NS = 3_600_000_000_000


def configured(max_size: int, warm_size: int) -> CachePolicy:
    return CachePolicy(
        version=1,
        root=Path("cache"),
        authority_environment="CAPSEM_TEST_CACHE_AUTHORITY",
        stages={
            "cargo": StagePolicy(
                path=Path("target/cargo"),
                description="cargo units",
                scope=CacheScope.DISK,
                warm_size_bytes=warm_size,
                max_size_bytes=max_size,
                prune_strategy=PruneStrategy.LRU,
                maximum_age_hours=720,
                cargo_target_roots=(Path("debug"), Path("llvm-cov-target/debug")),
                mutation_locks=(Path("debug/.cargo-lock"), Path("llvm-cov-target/debug/.cargo-lock")),
            )
        },
    )


def unit(root: Path, profile: str, name: str, digest: str, used_hours_ago: int, now_ns: int) -> list[Path]:
    """One unit's pieces as Cargo lays them out, each 10 bytes."""
    base = root / profile
    members = [
        base / f".fingerprint/{name}-{digest}/lib-{name}.json",
        base / f"build/{name}-{digest}/output",
        base / f"deps/lib{name.replace('-', '_')}-{digest}.rlib",
        base / f"deps/{name.replace('-', '_')}-{digest}.d",
        base / f"deps/.run-signed-{name}-{digest}-{'9' * 64}",
    ]
    stamp = now_ns - used_hours_ago * HOUR_NS
    for member in members:
        member.parent.mkdir(parents=True, exist_ok=True)
        member.write_bytes(b"x" * 10)
        os.utime(member, ns=(stamp, stamp))
    return members


def test_units_group_every_piece_of_one_compilation_and_rank_by_last_use(tmp_path: Path) -> None:
    now = 1_000 * HOUR_NS
    policy = configured(max_size=120, warm_size=70)
    paths = CachePaths(repository_root=tmp_path, policy=policy)
    root = paths.stage("cargo")
    old = unit(root, "debug", "capsem-core", OLD, used_hours_ago=48, now_ns=now)
    new = unit(root, "debug", "serde", NEW, used_hours_ago=1, now_ns=now)
    stale_coverage = unit(root, "llvm-cov-target/debug", "capsem-core", OLD, used_hours_ago=24, now_ns=now)
    (root / "debug/incremental/capsem_core-3k2j4h").mkdir(parents=True)
    (root / "debug/incremental/capsem_core-3k2j4h/session").write_bytes(b"x" * 10)
    uplifted = root / "debug/capsem"
    uplifted.write_bytes(b"x" * 10)
    (root / "debug/.cargo-lock").touch()

    inventory = scan_retention_inventory(paths, policy, now_ns=now)
    stage = inventory.stages[0]
    assert stage.logical_bytes == 170, "every byte in the stage is accounted exactly once"
    units = {entry.key: entry for entry in stage.entries}
    assert units[f"debug/{OLD}"].logical_bytes == 50
    grouped = [units[f"debug/{OLD}"].relative_path, *units[f"debug/{OLD}"].member_paths]
    assert sorted(root / path for path in grouped) == sorted(member.parent if ".fingerprint" in str(member) or "/build/" in str(member) else member for member in old)
    assert grouped[0].parts[1] == ".fingerprint", "the fingerprint goes first, so no output outlives it"

    plan = plan_prune(inventory, policy)
    removed = {action.path for action in plan.actions}
    assert all((member.parent if ".fingerprint" in str(member) or "/build/" in str(member) else member) in removed for member in old + stale_coverage)
    assert not any(str(member) in {str(path) for path in removed} for member in new), "the recently used unit stays"
    assert uplifted not in removed, "a path Cargo does not name by unit is never selected"
    assert plan.reclaim_bytes == 100
    assert not plan.violations

    apply_prune(paths, plan, reason="recover to warm")
    assert not (root / f"debug/.fingerprint/capsem-core-{OLD}").exists()
    assert all(member.exists() for member in new)
    assert uplifted.exists()


def test_a_building_cargo_keeps_every_unit(tmp_path: Path) -> None:
    now = 1_000 * HOUR_NS
    policy = configured(max_size=10, warm_size=5)
    paths = CachePaths(repository_root=tmp_path, policy=policy)
    root = paths.stage("cargo")
    unit(root, "debug", "capsem-core", OLD, used_hours_ago=48, now_ns=now)
    lock = root / "debug/.cargo-lock"
    lock.touch()
    with lock.open("rb") as descriptor:
        fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        inventory = scan_retention_inventory(paths, policy, now_ns=now)
    assert all(entry.protected for entry in inventory.stages[0].entries)
    plan = plan_prune(inventory, policy)
    assert not plan.actions
    # Enforcement refused a release run with only "cargo remains N bytes above
    # max size M": nothing said a live Cargo lock pinned every unit, or whose.
    [violation] = plan.violations
    assert "50 bytes are protected" in violation
    assert "debug/.cargo-lock" in violation
    if Path("/proc/locks").is_file():
        assert f"pid {os.getpid()}" in violation, "the holder is named, not just the lock"


def test_an_unpinned_stage_above_max_says_nothing_is_held(tmp_path: Path) -> None:
    now = 1_000 * HOUR_NS
    policy = configured(max_size=10, warm_size=5)
    paths = CachePaths(repository_root=tmp_path, policy=policy)
    root = paths.stage("cargo")
    (root / "debug").mkdir(parents=True)
    (root / "debug/capsem").write_bytes(b"x" * 40)  # uplifted: counted, never selected

    structural, over_max = plan_prune(
        scan_retention_inventory(paths, policy, now_ns=now), policy
    ).violations

    # Retention can never select an uplifted path, so the stage can never
    # reach warm: that is the cause, reported before the stage reaches max.
    assert structural.startswith("cargo can never recover to warm size 5")
    assert over_max.startswith("cargo remains 40 bytes above max size 10")
    assert "0 bytes are protected" in over_max and "no lock is held" in over_max
    for violation in (structural, over_max):
        assert "40 bytes are outside every generation" in violation
        assert "debug/capsem (40 bytes)" in violation, "the unselectable path is named"


def test_unselectable_bytes_within_warm_are_reported_not_refused(tmp_path: Path) -> None:
    now = 1_000 * HOUR_NS
    policy = configured(max_size=100, warm_size=50)
    paths = CachePaths(repository_root=tmp_path, policy=policy)
    root = paths.stage("cargo")
    (root / "debug").mkdir(parents=True)
    (root / "debug/capsem").write_bytes(b"x" * 40)

    inventory = scan_retention_inventory(paths, policy, now_ns=now)

    assert inventory.stages[0].unselectable_bytes == 40
    assert not plan_prune(inventory, policy).violations


def test_unaccounted_walk_does_not_search_every_unit_for_each_directory(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    root = tmp_path / "cargo"
    accounted = set()
    for index in range(100):
        member = root / "debug" / "deps" / f"unit-{index:04d}"
        member.parent.mkdir(parents=True, exist_ok=True)
        member.write_bytes(b"x")
        accounted.add(member)
    (root / "debug" / "uplifted").write_bytes(b"unowned")
    for index in range(100):
        unowned = root / "debug" / f"extra-{index:04d}"
        unowned.mkdir()
        (unowned / "bytes").write_bytes(b"z")

    original = Path.parents
    parent_lookups = 0

    def counted_parents(path: Path):
        nonlocal parent_lookups
        parent_lookups += 1
        return original.__get__(path, type(path))

    with monkeypatch.context() as patch:
        patch.setattr(Path, "parents", property(counted_parents))
        logical, _, _ = unaccounted_size(root, frozenset(accounted), set())
    assert logical == len(b"unowned") + 100
    assert parent_lookups < 500, "inventory must not compare every unowned subtree with every unit"


@pytest.mark.parametrize("value", ["../outside", "/tmp/outside", "."])
def test_cargo_target_roots_stay_inside_the_stage(value: str) -> None:
    with pytest.raises(ValueError):
        configured(10, 5).stages["cargo"].model_copy(update={}).model_validate(
            {**configured(10, 5).stages["cargo"].model_dump(), "cargo_target_roots": (Path(value),)}
        )


def test_cargo_units_and_a_retention_root_are_one_policy_not_two() -> None:
    stage = configured(10, 5).stages["cargo"]
    with pytest.raises(ValueError, match="cargo_target_roots"):
        StagePolicy.model_validate({**stage.model_dump(), "retention_root": Path("debug/incremental")})


def test_signed_copies_of_uplifted_binaries_are_reclaimable_generations(tmp_path: Path) -> None:
    # `run_signed.sh` keys each signed copy by the source's inode and ctime, so
    # every rebuild of an uplifted binary such as `debug/capsem-admin` leaves a
    # new copy beside it. None carried a unit hash: 1,610 copies (71 GB) sat
    # outside every unit and enforcement refused every compile on the machine.
    now = 1_000 * HOUR_NS
    policy = configured(max_size=60, warm_size=30)
    paths = CachePaths(repository_root=tmp_path, policy=policy)
    root = paths.stage("cargo")
    copies = {}
    for age, key in ((72, "a" * 64), (48, "b" * 64), (1, "c" * 64)):
        copy = root / f"debug/.run-signed-capsem-admin-{key}"
        receipt = root / f"debug/.run-signed/{key}"
        receipt.parent.mkdir(parents=True, exist_ok=True)
        copy.write_bytes(b"x" * 18)
        receipt.write_bytes(b"x" * 2)
        stamp = now - age * HOUR_NS
        for path in (copy, receipt):
            os.utime(path, ns=(stamp, stamp))
        copies[key] = (copy, receipt)
    staging = root / f"debug/.run-signed-capsem-admin-{'d' * 64}.tmp.4242"
    staging.write_bytes(b"x" * 10)
    abandoned = now - 96 * HOUR_NS
    os.utime(staging, ns=(abandoned, abandoned))

    inventory = scan_retention_inventory(paths, policy, now_ns=now)
    entries = {entry.key: entry for entry in inventory.stages[0].entries}
    old = entries[f"debug/signed/{'a' * 64}"]
    assert old.relative_path.parts[-2:] == (".run-signed", "a" * 64), "the receipt goes first"
    assert old.logical_bytes == 20

    removed = {root / action.path for action in plan_prune(inventory, policy).actions}
    for key in ("a" * 64, "b" * 64):
        assert set(copies[key]) <= removed
    assert staging in removed, "an abandoned staging copy is reclaimable too"
    assert not set(copies["c" * 64]) & removed, "the newest copy, the one being run, stays"
