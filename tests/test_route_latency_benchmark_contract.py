import importlib.util
from pathlib import Path

import pytest

from tests.ironbank.test_route_health import (
    HOT_ROUTE_MEASUREMENT_SAMPLES,
    HOT_ROUTE_WINDOW_SAMPLES,
    RouteTiming,
    _assert_hot_route_budget,
    _median_route_windows,
)

MODULE_PATH = Path(__file__).parent / "capsem-serial" / "test_route_latency_benchmark.py"
SPEC = importlib.util.spec_from_file_location("route_latency_benchmark", MODULE_PATH)
assert SPEC is not None
route_latency_benchmark = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(route_latency_benchmark)
_assert_route_contention_benchmark_budget = (
    route_latency_benchmark._assert_route_contention_benchmark_budget
)


def test_contention_benchmark_budget_gates_p99_not_single_tail_outlier() -> None:
    summary = {
        "samples": 160,
        "p95_ms": 1.2,
        "p99_ms": 1.5,
        "max_ms": 59.709,
        "service_cpu_s": 0.28,
    }
    gates = {
        "p95_ms_max": 15.0,
        "p99_ms_max": 40.0,
        "service_cpu_s_max": 0.34,
    }

    _assert_route_contention_benchmark_budget(summary, gates)


def test_contention_benchmark_budget_rejects_p99_regression() -> None:
    summary = {
        "samples": 160,
        "p95_ms": 1.2,
        "p99_ms": 42.0,
        "max_ms": 59.709,
        "service_cpu_s": 0.28,
    }
    gates = {
        "p95_ms_max": 15.0,
        "p99_ms_max": 40.0,
        "service_cpu_s_max": 0.34,
    }

    with pytest.raises(AssertionError):
        _assert_route_contention_benchmark_budget(summary, gates)


def test_hot_route_cpu_budget_scales_with_the_measurement_window() -> None:
    samples = [0.5] * HOT_ROUTE_MEASUREMENT_SAMPLES
    within_budget = RouteTiming(
        label="service /profiles/code/enforcement/info",
        samples_ms=samples,
        service_cpu_s=0.1,
        gateway_cpu_s=None,
    )
    over_budget = RouteTiming(
        label=within_budget.label,
        samples_ms=samples,
        service_cpu_s=0.121,
        gateway_cpu_s=None,
    )

    _assert_hot_route_budget(
        within_budget,
        path="/profiles/code/enforcement/info",
    )
    with pytest.raises(AssertionError, match="service CPU"):
        _assert_hot_route_budget(
            over_budget,
            path="/profiles/code/enforcement/info",
        )


def test_hot_route_latency_uses_the_configured_relative_factor() -> None:
    samples = [2.41] * HOT_ROUTE_MEASUREMENT_SAMPLES
    timing = RouteTiming(
        label="service /profiles/list",
        samples_ms=samples,
        service_cpu_s=0.1,
        gateway_cpu_s=None,
    )

    with pytest.raises(AssertionError, match="p95"):
        _assert_hot_route_budget(timing, path="/profiles/list")


def test_hot_route_cpu_uses_the_median_of_independent_windows() -> None:
    windows = [
        RouteTiming(
            label="service /profiles/list",
            samples_ms=[0.5] * HOT_ROUTE_WINDOW_SAMPLES,
            service_cpu_s=cpu,
            gateway_cpu_s=None,
        )
        for cpu in (0.1, 0.5, 0.09)
    ]

    combined = _median_route_windows(windows)

    assert len(combined.samples_ms) == HOT_ROUTE_MEASUREMENT_SAMPLES
    assert combined.service_cpu_s == 0.1
    assert combined.gateway_cpu_s is None
