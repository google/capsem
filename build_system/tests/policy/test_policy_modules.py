from __future__ import annotations

from datetime import UTC, datetime
from typing import cast

import pytest
from capsem_builder.policy.cachepolicy import (
    CacheLimits,
    CacheProduct,
    plan_reclaim,
)
from capsem_builder.policy.dockerpolicy import (
    BuildNetwork,
    ContainerNetwork,
    require_build_network,
    require_container_network,
)
from capsem_builder.policy.storagepolicyretention import (
    cache_violations,
    protect_generations,
    resource_decision,
    superseded_generations,
    validate_bounds,
)


def test_cache_policy_evicts_oldest_unprotected_product() -> None:
    limits = CacheLimits(maximum_count=1, maximum_age_seconds=100, maximum_bytes=20)
    products = (
        CacheProduct("old", 10, created_at=1, last_used_at=2),
        CacheProduct("new", 10, created_at=3, last_used_at=4),
    )

    assert plan_reclaim(products, limits, now=5).evict == ("old",)


def test_cache_policy_rejects_duplicate_keys() -> None:
    limits = CacheLimits(maximum_count=2, maximum_age_seconds=100, maximum_bytes=20)
    duplicate = CacheProduct("same", 1, created_at=1, last_used_at=1)

    with pytest.raises(ValueError, match="keys must be unique"):
        plan_reclaim((duplicate, duplicate), limits, now=2)


@pytest.mark.parametrize(
    ("field", "message"),
    [
        ("maximum_count", "maximum_count"),
        ("maximum_age_seconds", "maximum_age_seconds"),
        ("maximum_bytes", "maximum_bytes"),
    ],
)
def test_cache_limits_require_positive_bounds(field: str, message: str) -> None:
    values: dict[str, int | float] = {
        "maximum_count": 1,
        "maximum_age_seconds": 1,
        "maximum_bytes": 1,
    }
    values[field] = 0

    with pytest.raises(ValueError, match=message):
        CacheLimits(
            maximum_count=int(values["maximum_count"]),
            maximum_age_seconds=float(values["maximum_age_seconds"]),
            maximum_bytes=int(values["maximum_bytes"]),
        )


def test_cache_products_reject_invalid_identity_size_and_time() -> None:
    with pytest.raises(ValueError, match="non-empty"):
        CacheProduct("", 1, created_at=1, last_used_at=1)
    with pytest.raises(ValueError, match="negative"):
        CacheProduct("negative", -1, created_at=1, last_used_at=1)
    with pytest.raises(ValueError, match="timestamps"):
        CacheProduct("clock", 1, created_at=2, last_used_at=1)


def test_cache_policy_reports_unavoidable_protected_violations() -> None:
    limits = CacheLimits(maximum_count=1, maximum_age_seconds=1, maximum_bytes=1)
    protected = (
        CacheProduct("expired", 2, created_at=1, last_used_at=1, protected=True),
        CacheProduct("future", 2, created_at=10, last_used_at=10, protected=True),
    )

    plan = plan_reclaim(protected, limits, now=5)

    assert plan.evict == ()
    assert plan.violations == (
        "count 2 exceeds 1",
        "bytes 4 exceeds 1",
        "expired protected products: expired",
        "future-dated protected products: future",
    )
    with pytest.raises(ValueError, match="clock"):
        plan_reclaim((), limits, now=-1)


def test_cache_policy_expires_unprotected_products_before_lru_reclaim() -> None:
    limits = CacheLimits(maximum_count=2, maximum_age_seconds=1, maximum_bytes=10)
    expired = CacheProduct("expired", 1, created_at=1, last_used_at=1)

    assert plan_reclaim((expired,), limits, now=3).evict == ("expired",)


def test_docker_network_boundaries_reject_the_other_enum_family() -> None:
    assert require_build_network(BuildNetwork.NONE) == "none"
    assert require_container_network(ContainerNetwork.NONE) == "none"
    with pytest.raises(TypeError, match="BuildNetwork"):
        require_build_network(cast(BuildNetwork, ContainerNetwork.NONE))
    with pytest.raises(TypeError, match="ContainerNetwork"):
        require_container_network(cast(ContainerNetwork, BuildNetwork.NONE))


def test_storage_policy_keeps_pinned_generations_out_of_removal() -> None:
    generations = [
        {"ref": "current", "created": "2026-01-03T00:00:00Z", "size_bytes": 1},
        {"ref": "recent", "created": "2026-01-02T00:00:00Z", "size_bytes": 1},
        {"ref": "old", "created": "2026-01-01T00:00:00Z", "size_bytes": 1},
    ]
    retained, removable = superseded_generations(
        generations, keep="current", keep_previous=1
    )

    assert [row["ref"] for row in retained] == ["recent"]
    pinned, removable = protect_generations(retained, removable, {"old"})
    assert [row["ref"] for row in pinned] == ["recent", "old"]
    assert removable == []


def test_storage_policy_rejects_partial_or_boolean_bounds() -> None:
    with pytest.raises(ValueError, match="positive integers"):
        validate_bounds(
            "images",
            {
                "maximum_count": True,
                "maximum_age_hours": 1,
                "maximum_total_gib": 1,
            },
        )


@pytest.mark.parametrize(
    ("resource", "decision"),
    [
        ({"retention": "cache"}, "retain-cache"),
        ({"retention": "obsolete"}, "delete-obsolete"),
        (
            {"retention": "generational", "keep_previous": 2},
            "retain-current-and-2-previous",
        ),
        ({"retention": "release", "release_boundary": "binary"}, "release-binary"),
        (
            {"retention": "release", "release_boundaries": ["binary", "profile"]},
            "release-binary,profile",
        ),
    ],
)
def test_storage_policy_renders_each_retention_family(
    resource: dict[str, object], decision: str
) -> None:
    assert resource_decision(resource) == decision


def test_storage_policy_reports_expired_pinned_generation() -> None:
    resource = {
        "maximum_count": 2,
        "maximum_age_hours": 1,
        "maximum_total_gib": 1,
    }
    survivors = [
        {"ref": "current", "created": "2026-01-02T00:00:00Z", "size_bytes": 1},
        {"ref": "old", "created": "2026-01-01T00:00:00Z", "size_bytes": 1},
    ]

    assert cache_violations(
        resource,
        survivors,
        keep="current",
        now=datetime(2026, 1, 2, tzinfo=UTC),
    ) == ("expired protected images: old",)


def test_storage_policy_reports_count_bytes_and_future_bounds() -> None:
    resource = {
        "maximum_count": 1,
        "maximum_age_hours": 1,
        "maximum_total_gib": 1,
    }
    survivors = [
        {
            "ref": "current",
            "created": "2026-01-02T00:00:00Z",
            "size_bytes": 1024**3,
        },
        {
            "ref": "future",
            "created": "2026-01-03T00:00:00Z",
            "size_bytes": 1,
        },
    ]

    assert cache_violations(
        resource,
        survivors,
        keep="current",
        now=datetime(2026, 1, 2, tzinfo=UTC),
    ) == (
        "count 2 exceeds 1",
        f"bytes {1024**3 + 1} exceeds {1024**3}",
        "future-dated protected images: future",
    )
