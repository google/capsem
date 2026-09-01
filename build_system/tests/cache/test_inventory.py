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
