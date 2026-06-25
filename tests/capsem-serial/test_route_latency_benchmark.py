"""Host-side route latency/contention benchmark.

This is the release benchmark sibling of the Ironbank route-latency gate. It
uses the same black-box public route stimulus, then archives the exact numbers
so we can compare the disk-backed DB handle baseline against the upcoming
DB-owned memory/disk implementation.
"""

from __future__ import annotations

import json
import re
import time
from pathlib import Path

import pytest

from tests.ironbank.test_route_health import (
    route_timing_summary,
    run_concurrent_route_read_write_benchmark,
)


pytestmark = [pytest.mark.serial, pytest.mark.benchmark]

PROJECT_ROOT = Path(__file__).parent.parent.parent


def _project_version() -> str:
    cargo = PROJECT_ROOT / "Cargo.toml"
    match = re.search(r'^version\s*=\s*"([^"]+)"', cargo.read_text(), re.MULTILINE)
    return match.group(1) if match else "unknown"


def _save_benchmark(category: str, data: dict) -> Path:
    version = _project_version()
    out_dir = PROJECT_ROOT / "benchmarks" / category
    out_dir.mkdir(parents=True, exist_ok=True)
    out_path = out_dir / f"data_{version}.json"
    out_path.write_text(json.dumps(data, indent=2) + "\n")
    print(f"{category} benchmark saved to {out_path}")
    return out_path


def test_route_read_write_contention_benchmark() -> None:
    """Archive `/stats` route latency while profile mutation writes are active."""

    result = run_concurrent_route_read_write_benchmark(samples=160, mutation_repeats=8)
    summary = route_timing_summary(result.timing)
    actions = [row["action"] for row in result.writer_results]
    data = {
        "version": "0.1.0",
        "timestamp": time.time(),
        "scenario": "service_stats_reads_during_profile_mutation_writes",
        "reader": {
            "route": "/stats",
            "transport": "service_uds",
            "summary": summary,
        },
        "writer": {
            "route": "/profiles/code/mcp/default/edit",
            "transport": "service_uds",
            "writes": len(actions),
            "actions": actions,
            "unique_actions": sorted(set(actions)),
            "final_default_action": result.final_default_action,
            "final_default_rule_id": result.final_default_rule_id,
        },
        "gates": {
            "p95_ms_max": 15.0,
            "max_ms_max": 40.0,
            "service_cpu_s_max": 0.30,
        },
    }

    assert actions == ["allow", "ask", "block"] * 8
    assert result.final_default_action == actions[-1]
    assert result.final_default_rule_id
    assert summary["samples"] == 160
    assert summary["p95_ms"] <= data["gates"]["p95_ms_max"]
    assert summary["max_ms"] <= data["gates"]["max_ms_max"]
    assert summary["service_cpu_s"] <= data["gates"]["service_cpu_s_max"]

    path = _save_benchmark("route-latency", data)
    reloaded = json.loads(path.read_text())
    assert reloaded == data
