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
from helpers.collector_source import source_main
from helpers.http_measurement import MeasuredResponse

from benchmarks.helpers.route_measurements import Observations

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)
COLLECTOR = CONFIG.path(CONFIG.benchmark.run.collectors) / "routes"


def _collector() -> dict:
    return runpy.run_path(str(COLLECTOR))


def test_source_boundary_rejects_unmocked_collector_before_any_fixture():
    import pytest
    from helpers.collector_source import source_main
    called = []
    namespace = {'collect_new':json.loads,'main':lambda:called.append('native')}
    with pytest.raises(AssertionError,match='collect_new'):
        source_main(namespace,test_module=__name__)
    assert called == []


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

        def measure_request(self, *_args, **_kwargs):
            nonlocal active, peak
            if not self.warmed:
                self.warmed = True
                return MeasuredResponse(200, {}, b"{}", 1, 2, 3)
            with lock:
                active += 1
                peak = max(peak, active)
            time.sleep(0.002)
            with lock:
                active -= 1
            return MeasuredResponse(200, {}, b"{}", 1, 2, 3)

        def close(self):
            pass

    target = collector["Target"]("test", Transport, {}, {})
    latencies, cpu, requests_per_second = collector["sample_concurrent_route"](
        target, "/status", samples=24, concurrency=4
    )

    assert len(latencies.completion) == len(latencies.headers) == len(latencies.first_body_byte) == 24
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
        return Observations([1.0], [0.1], [0.2]), {}

    monkeypatch.setattr(sys, "argv", [str(COLLECTOR), "--samples", "1"])
    for key, value in {
        "ServiceInstance": lambda: service,
        "GatewayInstance": extra_gateway,
        "image_session": image_session,
        "psutil": SimpleNamespace(Process=lambda pid: pid),
        "seed_stopped_workspace": lambda *_, **__: None,
        "seed_metadata": lambda *_: None,
        "seed_logs": lambda *_: None,
        "collect_policy": lambda *_: {},
        "collect_lifecycle": lambda *_: {},
        "collect_networks": lambda *_: {},
        "collect_images": lambda *_: {},
        "collect_proxies": lambda *_: {},
        "collect_workload_controls": lambda *_: {},
        "collect_connection_errors": lambda *_: {},
        "collect_network_members": lambda *_: {},
        "collect_credentials": lambda *_: {},
        "collect_mcp_controls": lambda *_: {},
        "collect_preview_controls": lambda *_: {},
        "collect_run_purge": lambda *_: {},
        "collect_save_controls": lambda *_: {},
        "collect_ask_controls": lambda *_: {},
        "collect_reload_controls": lambda *_: {},
        "collect_pause_resume": lambda *_: {},
        "clone_benchmark_assets": lambda *args: tmp_path,
        "measured_cases": lambda: [module["RouteCase"]("GET", "/vms/{id}/exposures", "default")],
        "preflight": lambda *_: None,
        "sample_warm_route": warm,
        "sample_cold_route": lambda *_: Observations([1.0], [0.1], [0.2]),
        "sample_concurrent_route": lambda target, path, samples, *_: (Observations([1.0] * samples, [0.1] * samples, [0.2] * samples), {}, 1.0),
    }.items():
        monkeypatch.setitem(globals_, key, value)
    assert source_main(globals_,test_module=__name__) == 0
    assert calls == ["start", "product image", "service", "gateway", "stop"]
    assert json.loads(capsys.readouterr().out)["metrics"]


def test_preflight_checks_declared_method_and_real_payload_before_timing() -> None:
    import pytest

    collector = _collector()
    closed = []

    class Transport:
        def measure_request(self, *_args, **_kwargs):
            return MeasuredResponse(200, {}, b'{"events":[]}', 1, 2, 3)

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


def test_routes_refuse_partial_or_retried_measurement_receipts() -> None:
    import pytest
    collector = _collector()
    for receipt in (MeasuredResponse(200, {}, b"{}", 1, None, 3), MeasuredResponse(200, {}, b"{}", 1, 2, 3, attempts=2)):
        class Transport:
            def request(self, *_args, **_kwargs):
                raise AssertionError("functional retrying request must not be used")
            def measure_request(self, *_args, value=receipt, **kwargs):
                assert kwargs["timeout"] > 0 and kwargs["max_response_bytes"] > 0
                return value
            def close(self):
                pass
        target = collector["Target"]("test", Transport, {}, {})
        with pytest.raises(ValueError, match="receipt"):
            collector["sample_warm_route"](target, "/status", 1)


def test_route_raw_series_keep_expected_counts_and_clock_identity() -> None:
    import pytest

    from benchmarks.helpers.route_measurements import route_metrics
    observations = Observations()
    for i in (1, 2):
        observations.append(MeasuredResponse(200, {}, b"{}", i, i + 1, i + 2))
    observations.require_count(2)
    metrics = route_metrics("gateway", "GET /status [default]", observations, observations, {}, expected=2)
    for suffix in ("latency_ms", "headers_ms", "first_body_byte_ms", "cold_latency_ms", "cold_headers_ms", "cold_first_body_byte_ms"):
        assert len(metrics[f"gateway.GET /status [default].{suffix}"]["samples"]) == 2
    assert metrics["gateway.GET /status [default].headers_ms"]["samples"] == [1e-6, 2e-6]
    with pytest.raises(ValueError, match="count"):
        route_metrics("gateway", "/status", observations, None, {}, expected=3)


def test_strict_route_limits_are_owned_by_typed_config() -> None:
    import pytest
    from capsem_builder.gate.benchmarkschema import RouteLoadConfig
    from pydantic import ValidationError
    settings = _collector()["route_load_config"]()
    assert RouteLoadConfig(**settings).max_response_bytes == settings["max_response_bytes"]
    for field, invalid in (("timeout_seconds", 0), ("timeout_seconds", float("inf")), ("max_response_bytes", 0)):
        with pytest.raises(ValidationError):
            RouteLoadConfig(**{**settings, field: invalid})


def test_invalid_sample_count_fails_before_fixture_creation(monkeypatch, capsys) -> None:
    import pytest
    module = _collector()
    def forbidden():
        raise AssertionError("no child fixture may start")
    monkeypatch.setitem(module["main"].__globals__, "ServiceInstance", forbidden)
    for count in ("0", "-1"):
        monkeypatch.setattr(sys, "argv", ["routes", "--samples", count])
        with pytest.raises(SystemExit) as error:
            module["main"]()
        assert error.value.code == 2
        assert "samples must be positive" in capsys.readouterr().err
