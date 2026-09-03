"""The shared guest collector preserves hermetic controls and diagnostics."""

from __future__ import annotations

import os
import runpy
import shlex
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]
COLLECTOR = PROJECT_ROOT / "benchmarks" / "collectors" / "_guest"


def collector() -> dict:
    return runpy.run_path(str(COLLECTOR))


def test_guest_command_forwards_only_owned_benchmark_controls(monkeypatch) -> None:
    monkeypatch.setenv("CAPSEM_BENCH_CONCURRENCY", "1,8")
    monkeypatch.setenv("CAPSEM_BENCH_DURATION_S", "0.5")
    monkeypatch.setenv("CAPSEM_BENCH_MITM_TARGET", "https://example.invalid/a b")
    monkeypatch.setenv("UNRELATED_SECRET", "never-forward")

    command = collector()["guest_command"]("mitm-load")
    assert shlex.split(command) == [
        "env",
        "CAPSEM_BENCH_CONCURRENCY=1,8",
        "CAPSEM_BENCH_DURATION_S=0.5",
        "CAPSEM_BENCH_MITM_TARGET=https://example.invalid/a b",
        "capsem-bench",
        "mitm-load",
    ]
    assert os.environ["UNRELATED_SECRET"] not in command


def test_flatten_preserves_indexed_load_rows() -> None:
    flatten = collector()["flatten"]

    assert flatten(
        {
            "concurrency_levels": [
                {"concurrency": 1, "rps": 40.0, "p99_ms": 36.9},
                {"concurrency": 8, "rps": 120.0, "p99_ms": 80.0},
            ]
        }
    ) == {
        "concurrency_levels.0.concurrency": 1.0,
        "concurrency_levels.0.rps": 40.0,
        "concurrency_levels.0.p99_ms": 36.9,
        "concurrency_levels.1.concurrency": 8.0,
        "concurrency_levels.1.rps": 120.0,
        "concurrency_levels.1.p99_ms": 80.0,
    }
