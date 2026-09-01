"""Filesystem inventory reports deterministic stage-owned usage."""

import os
from pathlib import Path

from capsem_builder.cache.inventory import scan_inventory
from capsem_builder.cache.models import CachePolicy, PruneMethod, StagePolicy
from capsem_builder.cache.paths import CachePaths


def policy() -> CachePolicy:
    return CachePolicy(
        version=1,
        root=Path("cache"),
        minimum_free_bytes=1,
        stages={
            "objects": StagePolicy(
                path=Path("target/objects"),
                warning_bytes=10,
                soft_bytes=20,
                hard_bytes=30,
                prune=PruneMethod.LRU,
                maximum_age_hours=72,
            )
        },
    )


def test_inventory_counts_entries_and_does_not_follow_symlinks(tmp_path: Path) -> None:
    paths = CachePaths(repository_root=tmp_path, policy=policy())
    first = paths.stage("objects") / "first"
    first.mkdir(parents=True)
    (first / "payload").write_bytes(b"abc")
    outside = tmp_path / "outside"
    outside.write_bytes(b"outside-data")
    os.symlink(outside, first / "outside-link")

    report = scan_inventory(paths, policy(), now_ns=10)

    stage = report.stages[0]
    assert stage.stage_id == "objects"
    assert stage.entry_count == 1
    assert stage.logical_bytes == 3
    assert stage.entries[0].key == "first"
    assert report.logical_bytes == 3


def test_missing_stage_directory_is_an_empty_inventory(tmp_path: Path) -> None:
    report = scan_inventory(
        CachePaths(repository_root=tmp_path, policy=policy()), policy(), now_ns=10
    )

    assert report.stages[0].entry_count == 0
    assert report.stages[0].logical_bytes == 0


def test_allocated_bytes_count_cross_stage_hardlinks_once(tmp_path: Path) -> None:
    object_stage = policy().stages["objects"].model_copy(update={"path": Path("objects")})
    package_stage = object_stage.model_copy(update={"path": Path("target/packages")})
    configured = policy().model_copy(
        update={"stages": {"objects": object_stage, "packages": package_stage}}
    )
    paths = CachePaths(repository_root=tmp_path, policy=configured)
    payload = paths.stage("objects") / "payload"
    payload.parent.mkdir(parents=True)
    payload.write_bytes(b"same bytes")
    package = paths.stage("packages") / "Capsem.deb"
    package.parent.mkdir(parents=True)
    os.link(payload, package)

    report = scan_inventory(paths, configured, now_ns=10)
    stages = {stage.stage_id: stage for stage in report.stages}

    assert report.logical_bytes == 2 * len(b"same bytes")
    assert report.allocated_bytes == payload.stat().st_blocks * 512
    assert stages["objects"].allocated_bytes == report.allocated_bytes
    assert stages["packages"].allocated_bytes == 0


def test_inventory_reports_minimal_unclassified_roots(tmp_path: Path) -> None:
    paths = CachePaths(repository_root=tmp_path, policy=policy())
    paths.stage("objects").mkdir(parents=True)
    stray = paths.root / "target/stray/nested"
    stray.mkdir(parents=True)
    (stray / "payload").write_bytes(b"unmanaged")

    report = scan_inventory(paths, policy(), now_ns=10)

    assert [entry.relative_path for entry in report.unclassified] == [Path("target/stray")]
    assert report.unclassified[0].logical_bytes == len(b"unmanaged")
