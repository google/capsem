"""The official MITM scaling collector measures independent VM fanout."""

from __future__ import annotations

import runpy
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[1]
COLLECTOR = PROJECT_ROOT / "benchmarks" / "collectors" / "mitm-scale"


def collector() -> dict:
    return runpy.run_path(str(COLLECTOR))


def load_row(*, concurrency: int = 16, rps: float = 700.0, errors: int = 0) -> dict:
    return {
        "mitm_load": {
            "concurrency_levels": [
                {
                    "concurrency": concurrency,
                    "total_requests": 7000 + errors,
                    "errors": errors,
                    "rps": rps,
                    "p50_ms": 20.0,
                    "p95_ms": 30.0,
                    "p99_ms": 40.0,
                    "p999_ms": 50.0,
                }
            ]
        }
    }


def test_level_aggregates_rps_and_keeps_per_vm_latency_samples() -> None:
    metrics = collector()["level_metrics"](
        2,
        16,
        [load_row(rps=680.0), load_row(rps=720.0)],
    )

    assert metrics["levels.2.aggregate_rps"]["samples"] == [1400.0]
    assert metrics["levels.2.per_vm_rps"]["samples"] == [680.0, 720.0]
    assert metrics["levels.2.per_vm_p95_ms"]["samples"] == [30.0, 30.0]
    assert metrics["levels.2.requests"]["samples"] == [14000]
    assert metrics["levels.2.errors"]["samples"] == [0]


def test_level_refuses_failed_or_wrong_concurrency_results() -> None:
    level_metrics = collector()["level_metrics"]

    with pytest.raises(RuntimeError, match="failed requests"):
        level_metrics(1, 16, [load_row(errors=1)])
    with pytest.raises(RuntimeError, match="concurrency"):
        level_metrics(1, 16, [load_row(concurrency=8)])


def test_guest_command_fixes_one_cohort_per_vm() -> None:
    command = collector()["guest_load_command"](16, 10)
    assert "CAPSEM_BENCH_CONCURRENCY=16" in command
    assert "CAPSEM_BENCH_DURATION_S=10" in command
    assert command.endswith("capsem-bench mitm-load")


def test_fixture_pressure_is_namespaced_away_from_capsem_pressure() -> None:
    measured = {}
    collector()["add_resource_metrics"](
        measured,
        4,
        {"resources.total.cpu_s": {"unit": "seconds", "samples": [9.5]}},
        namespace="fixture",
    )
    assert measured == {
        "levels.4.fixture.resources.total.cpu_s": {
            "unit": "seconds",
            "samples": [9.5],
        }
    }
