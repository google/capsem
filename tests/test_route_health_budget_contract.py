from decimal import Decimal

from tests.ironbank.test_route_health import (
    HOT_ROUTE_MEASUREMENT_SAMPLES,
    RouteTiming,
    _assert_hot_route_budget,
    _assert_timing_budget,
    _cpu_delta_seconds,
    _hot_route_budget,
)


def test_route_health_budget_can_gate_p99_without_single_tail_outlier() -> None:
    timing = RouteTiming(
        label="service /stats during profile-mutation writes",
        samples_ms=[1.1] * 95 + [44.2],
        service_cpu_s=0.32,
        gateway_cpu_s=None,
    )

    _assert_timing_budget(timing, p95_ms=15.0, p99_ms=40.0, max_ms=None, cpu_s=0.34)


def test_route_health_budget_rejects_p99_regression() -> None:
    timing = RouteTiming(
        label="service /stats during profile-mutation writes",
        samples_ms=[1.1] * 94 + [41.0, 44.2],
        service_cpu_s=0.32,
        gateway_cpu_s=None,
    )

    try:
        _assert_timing_budget(timing, p95_ms=15.0, p99_ms=40.0, max_ms=None, cpu_s=0.34)
    except AssertionError:
        return

    raise AssertionError("p99 regression was not rejected")


def test_route_health_budget_rejects_cpu_regression() -> None:
    timing = RouteTiming(
        label="service /stats during profile-mutation writes",
        samples_ms=[1.1] * 160,
        service_cpu_s=0.36,
        gateway_cpu_s=None,
    )

    try:
        _assert_timing_budget(
            timing,
            p95_ms=15.0,
            p99_ms=40.0,
            max_ms=None,
            cpu_s=0.34,
        )
    except AssertionError:
        return

    raise AssertionError("service CPU regression was not rejected")


def test_cpu_accounting_delta_accepts_an_exact_budget_boundary() -> None:
    """Binary float subtraction must not turn an exact tick budget red."""
    raw_delta = 1.12 - 1.0
    assert raw_delta > 0.12

    timing = RouteTiming(
        label="service /profiles/list",
        samples_ms=[0.6] * HOT_ROUTE_MEASUREMENT_SAMPLES,
        service_cpu_s=_cpu_delta_seconds(after=Decimal("1.12"), before=Decimal("1.0")),
        gateway_cpu_s=None,
    )

    _assert_timing_budget(
        timing,
        p95_ms=2.0,
        p99_ms=5.0,
        max_ms=None,
        cpu_s=0.12,
        cpu_slack_s=0.0,
    )


def test_cpu_accounting_delta_rejects_the_next_accounted_tick() -> None:
    timing = RouteTiming(
        label="service /profiles/list",
        samples_ms=[0.6] * HOT_ROUTE_MEASUREMENT_SAMPLES,
        service_cpu_s=_cpu_delta_seconds(after=Decimal("1.13"), before=Decimal("1.0")),
        gateway_cpu_s=None,
    )

    try:
        _assert_timing_budget(
            timing,
            p95_ms=2.0,
            p99_ms=5.0,
            max_ms=None,
            cpu_s=0.12,
            cpu_slack_s=0.0,
        )
    except AssertionError:
        return

    raise AssertionError("the next CPU accounting tick was not rejected")


def test_hot_route_budget_ignores_one_host_scheduler_outlier() -> None:
    timing = RouteTiming(
        label="service /profiles/code/mcp/servers/local/tools/list",
        samples_ms=[0.2] * (HOT_ROUTE_MEASUREMENT_SAMPLES - 1) + [9.6],
        service_cpu_s=0.01,
        gateway_cpu_s=None,
    )

    _assert_hot_route_budget(timing, path="/profiles/code/mcp/servers/local/tools/list")


def test_hot_route_budget_rejects_a_sustained_tail_regression() -> None:
    outliers = 5
    timing = RouteTiming(
        label="service /profiles/code/mcp/servers/local/tools/list",
        samples_ms=[0.2] * (HOT_ROUTE_MEASUREMENT_SAMPLES - outliers) + [9.6] * outliers,
        service_cpu_s=0.01,
        gateway_cpu_s=None,
    )

    try:
        _assert_hot_route_budget(timing, path="/profiles/code/mcp/servers/local/tools/list")
    except AssertionError:
        return

    raise AssertionError("repeated hot-route tail regression was not rejected")


def test_gateway_status_budget_accounts_for_composite_service_work() -> None:
    """Gateway status aggregates several service-owned readiness projections."""
    direct_status_cpu_s = _hot_route_budget("/status")[2]
    gateway_status_cpu_s = _hot_route_budget("/status", gateway=True)[2]
    gateway_vm_list_cpu_s = _hot_route_budget("/vms/list", gateway=True)[2]

    assert gateway_status_cpu_s > direct_status_cpu_s
    assert gateway_status_cpu_s == gateway_vm_list_cpu_s
