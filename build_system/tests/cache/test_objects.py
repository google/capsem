"""Content-addressed objects deduplicate and verify every materialized view."""

import stat
from pathlib import Path

import pytest
from capsem_builder.cache.models import CachePolicy, CacheScope, PruneStrategy, StagePolicy
from capsem_builder.cache.objects import import_file, materialize, object_path
from capsem_builder.cache.paths import CachePaths


def paths(tmp_path: Path) -> CachePaths:
    stage = StagePolicy(
        path=Path("objects"),
        description="test cache",
        scope=CacheScope.DISK,
        warm_size_bytes=2,
        max_size_bytes=3,
        prune_strategy=PruneStrategy.NONE,
        maximum_age_hours=1,
    )
    return CachePaths(
        repository_root=tmp_path,
        policy=CachePolicy(version=1, root=Path("cache"), stages={"objects": stage}),
    )


def test_import_and_materialize_are_digest_verified_hardlinks(tmp_path: Path) -> None:
    cache = paths(tmp_path)
    source = tmp_path / "source"
    source.write_bytes(b"same immutable bytes")
    reference = import_file(cache, source)
    view = tmp_path / "view"
    materialize(cache, reference, view)

    assert view.read_bytes() == source.read_bytes()
    assert view.stat().st_ino == object_path(cache, reference).stat().st_ino
    assert stat.S_IMODE(view.stat().st_mode) & 0o222 == 0


def test_corrupted_existing_object_is_refused(tmp_path: Path) -> None:
    cache = paths(tmp_path)
    source = tmp_path / "source"
    source.write_bytes(b"expected")
    reference = import_file(cache, source)
    payload = object_path(cache, reference)
    payload.chmod(0o644)
    payload.write_bytes(b"corrupt!")

    with pytest.raises(ValueError, match="digest mismatch"):
        materialize(cache, reference, tmp_path / "view")
