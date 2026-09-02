"""Tool cache telemetry is typed, append-only, and stage-owned."""

import json
from pathlib import Path

from capsem_builder.cache.models import CachePolicy, CacheScope, PruneStrategy, StagePolicy
from capsem_builder.cache.paths import CachePaths
from capsem_builder.cache.telemetry import CacheTemperature, ReuseScope, record_use


def paths(tmp_path: Path) -> CachePaths:
    def stage(path: str) -> StagePolicy:
        return StagePolicy(
            path=Path(path),
            description="test cache",
            scope=CacheScope.DISK,
            warm_size_bytes=2,
            max_size_bytes=3,
            prune_strategy=PruneStrategy.LRU,
            maximum_age_hours=1,
        )

    policy = CachePolicy(
        version=1,
        root=Path("cache"),
        authority_environment="CAPSEM_TEST_CACHE_AUTHORITY",
        stages={"python-uv": stage("tools/python/uv"), "state": stage("state")},
    )
    return CachePaths(repository_root=tmp_path, policy=policy)


def test_records_cold_then_warm_generation_without_recursive_sizing(tmp_path: Path) -> None:
    cache = paths(tmp_path)
    cold = record_use(cache, "python-uv", tool="uv", key="lock", scope=ReuseScope.GENERATION)
    payload = cache.stage("python-uv") / "wheel"
    payload.parent.mkdir(parents=True)
    payload.write_bytes(b"abc")
    warm = record_use(cache, "python-uv", tool="uv", key="lock", scope=ReuseScope.GENERATION)

    assert cold.temperature is CacheTemperature.COLD and cold.observed_bytes is None
    assert warm.temperature is CacheTemperature.WARM and warm.observed_bytes is None
    rows = [
        json.loads(line) for line in (cache.stage("state") / "usage.jsonl").read_text().splitlines()
    ]
    assert [row["temperature"] for row in rows] == ["cold", "warm"]
    assert {row["schema_id"] for row in rows} == {"capsem.cache-use.v2"}


def test_keyed_probe_does_not_mistake_another_generation_for_a_hit(tmp_path: Path) -> None:
    cache = paths(tmp_path)
    other = cache.stage("python-uv") / "other-generation/payload"
    other.parent.mkdir(parents=True)
    other.write_bytes(b"warm")

    observed = record_use(
        cache,
        "python-uv",
        tool="uv",
        key="new-lock",
        scope=ReuseScope.GENERATION,
        probe=cache.stage("python-uv") / "new-generation",
    )

    assert observed.temperature is CacheTemperature.COLD


def test_shared_pool_ignores_runtime_socket_when_deciding_warmth(tmp_path: Path) -> None:
    cache = paths(tmp_path)
    stage = cache.stage("python-uv")
    stage.mkdir(parents=True)
    stage.joinpath("cache.sock").touch()

    cold = record_use(
        cache,
        "python-uv",
        tool="compiler",
        key="source",
        scope=ReuseScope.SHARED,
        ignored_names=("cache.sock",),
    )
    stage.joinpath("object").write_bytes(b"compiled")
    warm = record_use(
        cache,
        "python-uv",
        tool="compiler",
        key="source",
        scope=ReuseScope.SHARED,
        ignored_names=("cache.sock",),
    )

    assert cold.temperature is CacheTemperature.COLD
    assert warm.temperature is CacheTemperature.WARM
