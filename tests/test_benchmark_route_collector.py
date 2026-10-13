"""Contracts for the unified HTTP route benchmark collector."""

from __future__ import annotations

import runpy
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]


def _collector() -> dict:
    return runpy.run_path(str(PROJECT_ROOT / "benchmarks" / "collectors" / "routes"))


def test_every_measured_route_resolves_to_a_concrete_fixture_request() -> None:
    collector = _collector()
    request_path = collector["request_path"]

    unresolved = {
        route: request_path(route)
        for route in collector["measured_routes"]()
        if "{" in request_path(route) or "}" in request_path(route)
    }

    assert not unresolved, (
        "the benchmark sent route templates literally, so a fast 404 replaced "
        f"the intended latency measurement: {unresolved}"
    )
