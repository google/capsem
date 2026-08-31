import importlib.util
from pathlib import Path
from typing import cast

import psutil
import pytest

from tests.ironbank import test_route_health as route_health
from tests.ironbank.test_route_health import (
    HOT_ROUTE_MEASUREMENT_SAMPLES,
    HOT_ROUTE_REFERENCE_SAMPLES,
    HOT_ROUTE_WINDOW_SAMPLES,
    HOT_ROUTE_WINDOWS,
    RouteTiming,
    _assert_hot_route_budget,
    _measure_route_windows,
    _median_route_windows,
    _scaled_hot_route_budget,
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


def test_windowed_route_measurement_uses_independent_reference_windows(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    cpu_windows = iter((0.1, 0.5, 0.09))
    sample_counts: list[int] = []

    def measure_route(
        label: str,
        call: object,
        *,
        service_proc: psutil.Process,
        gateway_proc: psutil.Process | None = None,
        samples: int = 64,
    ) -> RouteTiming:
        del call, service_proc, gateway_proc
        sample_counts.append(samples)
        return RouteTiming(
            label=label,
            samples_ms=[0.5] * samples,
            service_cpu_s=next(cpu_windows),
            gateway_cpu_s=None,
        )

    monkeypatch.setattr(route_health, "_measure_route", measure_route)
    timing = _measure_route_windows(
        "service /vms/list",
        lambda: None,
        service_proc=cast(psutil.Process, object()),
        samples=HOT_ROUTE_REFERENCE_SAMPLES,
    )

    assert sample_counts == [HOT_ROUTE_REFERENCE_SAMPLES] * HOT_ROUTE_WINDOWS
    assert len(timing.samples_ms) == HOT_ROUTE_REFERENCE_SAMPLES * HOT_ROUTE_WINDOWS
    assert timing.service_cpu_s == 0.1


def test_reference_route_budget_uses_the_configured_regression_factor() -> None:
    _p95_ms, _p99_ms, cpu_s = _scaled_hot_route_budget(
        "/vms/list",
        gateway=True,
        samples=HOT_ROUTE_REFERENCE_SAMPLES,
    )

    assert cpu_s == pytest.approx(0.14 * route_health.HOT_ROUTE_REGRESSION_FACTOR)
