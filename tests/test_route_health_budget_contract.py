from tests.ironbank.test_route_health import RouteTiming, _assert_timing_budget


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
