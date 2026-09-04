"""Live verdict caching is exact, typed, bounded, and fail closed."""

from pathlib import Path

from capsem_builder.cache.contract import CacheScope, PruneStrategy
from capsem_builder.cache.models import CachePolicy, StagePolicy
from capsem_builder.cache.paths import CachePaths
from capsem_builder.cache.verdicts import record_clean, reusable, subject_digest

HOUR_NS = 3_600_000_000_000


def _paths(tmp_path: Path) -> CachePaths:
    return CachePaths(
        repository_root=tmp_path,
        policy=CachePolicy(
            version=1,
            root=Path("cache"),
            authority_environment="CAPSEM_TEST_CACHE_AUTHORITY",
            stages={
                "audit-results": StagePolicy(
                    path=Path("tools/audits"),
                    description="test live verdicts",
                    scope=CacheScope.DISK,
                    warm_size_bytes=1,
                    max_size_bytes=2,
                    prune_strategy=PruneStrategy.LRU,
                    maximum_age_hours=1,
                )
            },
        ),
    )


def test_exact_clean_verdict_is_reused_only_within_its_age(tmp_path: Path) -> None:
    paths = _paths(tmp_path)
    digest = subject_digest(b"locked graph")
    record_clean(
        paths,
        stage_id="audit-results",
        owner="pnpm",
        digest=digest,
        messages=("clean",),
        now_ns=HOUR_NS,
    )

    assert reusable(
        paths,
        stage_id="audit-results",
        owner="pnpm",
        digest=digest,
        now_ns=2 * HOUR_NS,
    ) is not None
    assert reusable(
        paths,
        stage_id="audit-results",
        owner="pnpm",
        digest=subject_digest(b"changed graph"),
        now_ns=2 * HOUR_NS,
    ) is None
    assert reusable(
        paths,
        stage_id="audit-results",
        owner="pnpm",
        digest=digest,
        now_ns=2 * HOUR_NS + 1,
    ) is None


def test_corrupt_or_future_verdict_is_never_reused(tmp_path: Path) -> None:
    paths = _paths(tmp_path)
    digest = subject_digest(b"graph")
    record = record_clean(
        paths,
        stage_id="audit-results",
        owner="pnpm",
        digest=digest,
        messages=("clean",),
        now_ns=2 * HOUR_NS,
    )
    target = paths.stage("audit-results") / record.owner / f"{digest}.json"

    assert reusable(
        paths,
        stage_id="audit-results",
        owner="pnpm",
        digest=digest,
        now_ns=HOUR_NS,
    ) is None
    target.write_text("not json", encoding="utf-8")
    assert reusable(
        paths,
        stage_id="audit-results",
        owner="pnpm",
        digest=digest,
        now_ns=3 * HOUR_NS,
    ) is None
