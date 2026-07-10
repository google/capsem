import pytest
import importlib.util
from pathlib import Path

MODULE_PATH = (
    Path(__file__).parent
    / "capsem-serial"
    / "test_route_latency_benchmark.py"
)
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
