"""Contracts for the unified HTTP route benchmark collector."""

from __future__ import annotations

import json
import runpy
import threading
import time
from pathlib import Path

from capsem_builder.gate import config as gate_config

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)
COLLECTOR = CONFIG.path(CONFIG.benchmark.run.collectors) / "routes"


def _collector() -> dict:
    return runpy.run_path(str(COLLECTOR))


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


def test_managed_routes_use_repeatable_authenticated_posts() -> None:
    collector = _collector()
    request_path = collector["request_path"]
    request_input = collector["request_input"]

    close_path = request_path("/vms/managed/{request_id}/close")
    method, body = request_input(close_path)
    assert method == "POST"
    assert json.loads(body) == {
        "ownership_token": "af" * 32,
        "outcome": "cancelled",
    }

    claim_path = request_path("/vms/managed/{request_id}/claim")
    method, body = request_input(claim_path)
    assert method == "POST"
    assert json.loads(body) == {"ownership_token": "af" * 32}


def test_concurrent_route_samples_real_overlap_and_rps() -> None:
    collector = _collector()
    active = 0
    peak = 0
    lock = threading.Lock()

    class Transport:
        def __init__(self):
            self.warmed = False

        def request(self, *_args, **_kwargs):
            nonlocal active, peak
            if not self.warmed:
                self.warmed = True
                return 200, {}, b"{}"
            with lock:
                active += 1
                peak = max(peak, active)
            time.sleep(0.002)
            with lock:
                active -= 1
            return 200, {}, b"{}"

        def close(self):
            pass

    target = collector["Target"]("test", Transport, {}, {})
    latencies, cpu, requests_per_second = collector["sample_concurrent_route"](
        target, "/status", samples=24, concurrency=4
    )

    assert len(latencies) == 24
    assert peak == 4
    assert cpu == {}
    assert requests_per_second > 0


def test_route_load_settings_cover_real_concurrency() -> None:
    collector = _collector()
    settings = collector["route_load_config"]()

    assert settings["concurrency"] > 1
    assert settings["requests"] >= settings["concurrency"]
    assert settings["quick_requests"] >= settings["concurrency"]
