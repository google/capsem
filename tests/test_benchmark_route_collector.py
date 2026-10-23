"""Contracts for the unified HTTP route benchmark collector."""

from __future__ import annotations

import json
import runpy
import sys
import threading
import time
from contextlib import contextmanager
from pathlib import Path
from types import SimpleNamespace

from capsem_builder.gate import config as gate_config

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)
COLLECTOR = CONFIG.path(CONFIG.benchmark.run.collectors) / "routes"


def _collector() -> dict:
    return runpy.run_path(str(COLLECTOR))


def test_every_measured_route_resolves_to_a_concrete_fixture_request() -> None:
    collector = _collector()

    assert all("{" not in case.resolve("live-owner", "network-id") for case in collector["measured_cases"]())


def test_managed_routes_use_repeatable_authenticated_posts() -> None:
    collector = _collector()
    request_input = collector["request_input"]

    close_path = collector["RouteCase"]("POST", "/vms/managed/{request_id}/close", "closed-tombstone").resolve(None)
    method, body = request_input(close_path)
    assert method == "POST"
    assert json.loads(body) == {
        "ownership_token": "af" * 32,
        "outcome": "cancelled",
    }

    claim_path = collector["RouteCase"]("POST", "/vms/managed/{request_id}/claim", "closed-tombstone").resolve(None)
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


def test_routes_reuse_owned_gateway_and_default_product_fixture(monkeypatch, tmp_path, capsys):
    module = _collector()
    globals_ = module["main"].__globals__
    (tmp_path / "gateway.port").write_text("12345")
    (tmp_path / "gateway.token").write_text("private-token")
    (tmp_path / "gateway.pid").write_text("4242")
    calls = []
    service = SimpleNamespace(
        home_dir=tmp_path, tmp_dir=tmp_path, uds_path=tmp_path / "service.sock",
        proc=SimpleNamespace(pid=123), start=lambda: calls.append("start"),
        stop=lambda: calls.append("stop"),
    )

    def extra_gateway(*_):
        raise AssertionError("a second gateway must not be built or spawned")

    @contextmanager
    def image_session(owner, directory, name):
        assert owner is service and directory == tmp_path / "route-registry"
        assert name == "route-live-owner"
        calls.append("product image")
        yield "live-id"

    def warm(target, path, samples, response):
        assert path == "/vms/live-id/exposures"
        if target.label == "gateway":
            assert target.headers == {"Authorization": "Bearer private-token"}
            assert target.cpu_processes == {"gateway": 4242, "downstream_service": 123}
        calls.append(target.label)
        return [1.0], {}

    monkeypatch.setattr(sys, "argv", [str(COLLECTOR), "--samples", "1"])
    for key, value in {
        "ServiceInstance": lambda: service,
        "GatewayInstance": extra_gateway,
        "image_session": image_session,
        "psutil": SimpleNamespace(Process=lambda pid: pid),
        "seed_stopped_workspace": lambda *_, **__: None,
        "seed_metadata": lambda *_: None,
        "seed_logs": lambda *_: None,
        "clone_benchmark_assets": lambda *args: tmp_path,
        "measured_cases": lambda: [module["RouteCase"]("GET", "/vms/{id}/exposures", "default")],
        "preflight": lambda *_: None,
        "sample_warm_route": warm,
        "sample_cold_route": lambda *_: [1.0],
        "sample_concurrent_route": lambda *_: ([1.0], {}, 1.0),
    }.items():
        monkeypatch.setitem(globals_, key, value)
    assert module["main"]() == 0
    assert calls == ["start", "product image", "service", "gateway", "stop"]
    assert json.loads(capsys.readouterr().out)["metrics"]


def test_preflight_checks_declared_method_and_real_payload_before_timing() -> None:
    import pytest

    collector = _collector()
    closed = []

    class Transport:
        def request(self, *_args, **_kwargs):
            return 200, {}, b'{"events":[]}'

        def close(self):
            closed.append(True)

    target = collector["Target"]("test", Transport, {}, {})
    case = collector["RouteCase"]("GET", "/vms/{id}/timeline", "recorded-session")
    with pytest.raises(AssertionError, match="must not be empty"):
        collector["preflight"](target, [case], None)
    case = collector["RouteCase"]("POST", "/vms/{id}/timeline", "recorded-session")
    with pytest.raises(ValueError, match="method differs"):
        collector["preflight"](target, [case], None)
    assert len(closed) == 2


def test_all_configured_cases_use_the_declared_http_method() -> None:
    collector = _collector()
    for case in collector["measured_cases"]():
        assert collector["request_input"](case.resolve("live-id", "network-id"))[0] == case.method
