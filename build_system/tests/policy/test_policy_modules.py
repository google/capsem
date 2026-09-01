from __future__ import annotations

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


def test_cache_policy_evicts_oldest_unprotected_product() -> None:
    limits = CacheLimits(maximum_count=1, maximum_age_seconds=100, maximum_bytes=20)
    products = (
        CacheProduct(key="old", size_bytes=10, created_at=1, last_used_at=2),
        CacheProduct(key="new", size_bytes=10, created_at=3, last_used_at=4),
    )

    assert plan_reclaim(products, limits, now=5).evict == ("old",)


def test_cache_policy_rejects_duplicate_keys() -> None:
    limits = CacheLimits(maximum_count=2, maximum_age_seconds=100, maximum_bytes=20)
    duplicate = CacheProduct(key="same", size_bytes=1, created_at=1, last_used_at=1)

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
    with pytest.raises(ValueError, match="at least 1 character"):
        CacheProduct(key="", size_bytes=1, created_at=1, last_used_at=1)
    with pytest.raises(ValueError, match="greater than or equal to 0"):
        CacheProduct(key="negative", size_bytes=-1, created_at=1, last_used_at=1)
    with pytest.raises(ValueError, match="last use cannot precede"):
        CacheProduct(key="clock", size_bytes=1, created_at=2, last_used_at=1)


def test_cache_policy_reports_unavoidable_protected_violations() -> None:
    limits = CacheLimits(maximum_count=1, maximum_age_seconds=1, maximum_bytes=1)
    protected = (
        CacheProduct(
            key="expired", size_bytes=2, created_at=1, last_used_at=1, protected=True
        ),
        CacheProduct(
            key="future", size_bytes=2, created_at=10, last_used_at=10, protected=True
        ),
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
    expired = CacheProduct(key="expired", size_bytes=1, created_at=1, last_used_at=1)

    assert plan_reclaim((expired,), limits, now=3).evict == ("expired",)


def test_docker_network_boundaries_reject_the_other_enum_family() -> None:
    assert require_build_network(BuildNetwork.NONE) == "none"
    assert require_container_network(ContainerNetwork.NONE) == "none"
    with pytest.raises(TypeError, match="BuildNetwork"):
        require_build_network(cast(BuildNetwork, ContainerNetwork.NONE))
    with pytest.raises(TypeError, match="ContainerNetwork"):
        require_container_network(cast(ContainerNetwork, BuildNetwork.NONE))
