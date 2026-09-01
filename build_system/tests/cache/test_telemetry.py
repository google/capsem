"""Tool cache telemetry is typed, append-only, and stage-owned."""

import json
from pathlib import Path

from capsem_builder.cache.models import CachePolicy, PruneMethod, StagePolicy
from capsem_builder.cache.paths import CachePaths
from capsem_builder.cache.telemetry import CacheOutcome, record_use


def paths(tmp_path: Path) -> CachePaths:
    def stage(path: str) -> StagePolicy:
        return StagePolicy(
            path=Path(path),
            warning_bytes=1,
            soft_bytes=2,
            hard_bytes=3,
            prune=PruneMethod.LRU,
            maximum_age_hours=1,
        )
    policy = CachePolicy(
        version=1,
        root=Path("cache"),
        minimum_free_bytes=1,
        stages={"python-uv": stage("tools/python/uv"), "state": stage("state")},
    )
    return CachePaths(repository_root=tmp_path, policy=policy)


def test_records_miss_then_hit_with_size(tmp_path: Path) -> None:
    cache = paths(tmp_path)
    miss = record_use(cache, "python-uv", tool="uv", key="lock")
    payload = cache.stage("python-uv") / "wheel"
    payload.parent.mkdir(parents=True)
    payload.write_bytes(b"abc")
    hit = record_use(cache, "python-uv", tool="uv", key="lock")

    assert miss.outcome is CacheOutcome.MISS and miss.logical_bytes == 0
    assert hit.outcome is CacheOutcome.HIT and hit.logical_bytes >= 3
    rows = [json.loads(line) for line in (cache.stage("state") / "usage.jsonl").read_text().splitlines()]
    assert [row["outcome"] for row in rows] == ["miss", "hit"]
