"""Every storage mechanism is hidden behind the same typed cache API."""

from pathlib import Path

from capsem_builder.cache.api import CacheOperation, CacheRequest
from capsem_builder.cache.models import CachePolicy, CacheScope, PruneStrategy, StagePolicy
from capsem_builder.cache.paths import CachePaths
from capsem_builder.cache.registry import CacheRegistry


def policy() -> CachePolicy:
    return CachePolicy(
        version=1,
        root=Path("cache"),
        stages={
            "objects": StagePolicy(
                description="test objects",
                scope=CacheScope.DISK,
                path=Path("objects"),
                warm_size_bytes=2,
                max_size_bytes=3,
                prune_strategy=PruneStrategy.LRU,
                maximum_age_hours=1,
            )
        },
    )


def test_registry_resolves_contract_without_exposing_a_path(tmp_path: Path) -> None:
    configured = policy()
    registry = CacheRegistry(CachePaths(repository_root=tmp_path, policy=configured), configured)

    contract = registry.contract("objects")

    assert contract.model_dump(mode="json") == {
        "description": "test objects",
        "scope": "disk",
        "max_size_bytes": 3,
        "warm_size_bytes": 2,
        "prune_strategy": "lru",
    }


def test_registry_prunes_a_disk_cache_through_the_common_request(tmp_path: Path) -> None:
    configured = policy()
    paths = CachePaths(repository_root=tmp_path, policy=configured)
    for name in ("old", "new"):
        payload = paths.stage("objects") / name / "payload"
        payload.parent.mkdir(parents=True)
        payload.write_bytes(b"xx")
    registry = CacheRegistry(paths, configured)

    (result,) = registry.mutate(
        CacheRequest(
            operation=CacheOperation.ENFORCE,
            cache_id="objects",
            apply=True,
            reason="test maximum",
        )
    )

    assert result.applied and result.action_count == 1
    assert result.after_size_bytes == configured.stages["objects"].warm_size_bytes
