"""Gateway-owned artifacts never create successful service-UDS samples."""
from __future__ import annotations

import copy
import json
from pathlib import Path

import pytest
from helpers.http_measurement import MeasuredResponse
from helpers.http_transport import Transport as HTTPTransport

from benchmarks.helpers.route_cases import RouteCase, measured_cases
from benchmarks.helpers.route_gateway import GATEWAY_RESPONSES, expected_artifact, validate_gateway
from benchmarks.helpers.route_requests import Target, request

ROOT = Path(__file__).resolve().parents[1]


def headers(response: str) -> dict[str, str]:
    if response in {"health", "openapi"}:
        return {"content-type": "application/json"}
    return {
        "content-type": "text/html; charset=utf-8" if response == "html" else "text/javascript; charset=utf-8",
        "x-content-type-options": "nosniff", "x-frame-options": "DENY",
        "referrer-policy": "no-referrer", "cache-control": "no-store",
        "cross-origin-opener-policy": "same-origin",
        "content-security-policy": "default-src 'none'; script-src 'self'; connect-src 'self'; style-src 'unsafe-inline'; form-action http://*.localhost:19444; frame-ancestors 'none'; base-uri 'none'",
    }


def test_gateway_only_cases_are_explicit_and_never_selected_for_service() -> None:
    cases = measured_cases()
    owned = [case for case in cases if case.target_scope == "gateway"]
    assert {case.path: case.response for case in owned} == GATEWAY_RESPONSES
    assert len(cases) == 65 and len({(case.method, case.path) for case in cases}) == 59
    for case in owned:
        assert not case.supports_target("service") and case.supports_target("gateway")
        assert not case.live and "{" not in case.resolve(None)
        with pytest.raises(ValueError, match="gateway"):
            RouteCase("GET", case.path, "bad")
    with pytest.raises(ValueError, match="scope"):
        RouteCase("GET", "/status", "bad", target_scope="typo")


def test_first_party_static_bytes_and_every_security_header_are_required() -> None:
    for response in ("html", "javascript"):
        body = expected_artifact(response)
        validate_gateway(response, body, headers(response), "/tmp/service.sock")
        with pytest.raises(AssertionError):
            validate_gateway(response, body + b"injected", headers(response), "/tmp/service.sock")
        for key in headers(response):
            damaged = headers(response)
            damaged[key] = "unsafe"
            with pytest.raises(AssertionError):
                validate_gateway(response, body, damaged, "/tmp/service.sock")
        with pytest.raises(AssertionError):
            validate_gateway(response, body, {**headers(response), "content-encoding": "gzip"}, "/tmp/service.sock")


def test_health_requires_exact_source_version_socket_and_true_boolean() -> None:
    import tomllib
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    payload = {"ok": True, "version": version, "service_socket": "/tmp/service.sock"}
    validate_gateway("health", json.dumps(payload).encode(), headers("health"), "/tmp/service.sock")
    for key, wrong in (("ok", 1), ("version", "old"), ("service_socket", "/tmp/other.sock")):
        with pytest.raises(AssertionError):
            validate_gateway("health", json.dumps({**payload, key: wrong}).encode(), headers("health"), "/tmp/service.sock")


def test_openapi_compares_complete_selected_operations_schemas_and_version() -> None:
    body = (ROOT / "sdk/specification/openapi.json").read_bytes()
    validate_gateway("openapi", body, headers("openapi"), "/tmp/service.sock")
    source = json.loads(body)
    for key in ("paths", "components", "info", "openapi"):
        damaged = copy.deepcopy(source)
        damaged[key] = {} if isinstance(source[key], dict) else "old"
        with pytest.raises(AssertionError):
            validate_gateway("openapi", json.dumps(damaged).encode(), headers("openapi"), "/tmp/service.sock")


def test_gateway_request_cannot_send_artifact_to_service_or_accept_404() -> None:
    class Transport(HTTPTransport):
        def __init__(self):
            super().__init__()
        def measure_request(self, *_args, **_kwargs):
            return MeasuredResponse(404, {}, b"missing", 1, 2, 3)
    transport = Transport()
    with pytest.raises(ValueError, match="gateway"):
        request(transport, Target("service", lambda: transport, {}, {}), "/", "health")
    with pytest.raises(RuntimeError, match="HTTP 404"):
        request(transport, Target("gateway", lambda: transport, {}, {}, service_socket="/tmp/service.sock"), "/", "health")


def test_preflight_never_opens_gateway_cases_over_service_uds() -> None:
    import runpy
    collector = runpy.run_path(str(ROOT / "benchmarks/collectors/routes"))
    class NoRequest(HTTPTransport):
        def __init__(self):
            super().__init__()
        def measure_request(self, *_args, **_kwargs):
            raise AssertionError("must never request a gateway artifact through service")
    owned = [case for case in measured_cases() if case.target_scope == "gateway"]
    collector["preflight"](Target("service", NoRequest, {}, {}), owned, None)


def test_main_emits_only_gateway_series_for_gateway_owned_cases(monkeypatch, tmp_path, capsys) -> None:
    import runpy
    import sys
    from types import SimpleNamespace

    from benchmarks.helpers.route_measurements import Observations
    module = runpy.run_path(str(ROOT / "benchmarks/collectors/routes"))
    calls = []
    service = SimpleNamespace(home_dir=tmp_path, tmp_dir=tmp_path, uds_path=tmp_path / "service.sock", proc=SimpleNamespace(pid=10), start=lambda: None, stop=lambda: None)
    cases = [RouteCase("GET", "/", "gateway-static", response="health", target_scope="gateway"), RouteCase("GET", "/status", "default")]
    def warm(target, path, *_):
        calls.append((target.label, path))
        assert target.label != "gateway" or target.service_socket == str(service.uds_path)
        return Observations([1.0], [0.1], [0.2]), {}
    for key, value in {
        "ServiceInstance": lambda: service,
        "service_gateway": lambda _: SimpleNamespace(port=12345, pid=20, token="synthetic-token"),
        "psutil": SimpleNamespace(Process=lambda pid: pid),
        "collect_images": lambda *_: {},
        "clone_benchmark_assets": lambda *_: tmp_path,
        "seed_stopped_workspace": lambda *_, **__: None,
        "seed_metadata": lambda *_: None, "seed_logs": lambda *_: None,
        "measured_cases": lambda: cases, "preflight": lambda *_: None,
        "sample_warm_route": warm,
        "sample_cold_route": lambda *_: Observations([1.0], [0.1], [0.2]),
        "sample_concurrent_route": lambda _, __, count, *args: (Observations([1.0] * count, [0.1] * count, [0.2] * count), {}, 1.0),
    }.items():
        monkeypatch.setitem(module["main"].__globals__, key, value)
    monkeypatch.setattr(sys, "argv", ["routes", "--samples", "1"])
    assert module["main"]() == 0
    assert calls == [("gateway", "/"), ("service", "/status"), ("gateway", "/status")]
    metrics = json.loads(capsys.readouterr().out)["metrics"]
    assert any(key.startswith("gateway.GET / [") for key in metrics)
    assert not any(key.startswith("service.GET / [") for key in metrics)


def test_typed_config_rejects_unknown_scope_and_gateway_case_contract_mismatch() -> None:
    from capsem_builder.gate.benchmarkschema import RouteCaseConfig
    from pydantic import ValidationError
    with pytest.raises(ValidationError):
        RouteCaseConfig.model_validate({"method": "GET", "path": "/", "case": "bad", "target_scope": "service"})
    with pytest.raises(ValueError, match="gateway"):
        RouteCase("GET", "/health", "bad", target_scope="gateway", response="html")
    for policy in (headers("html")["content-security-policy"].replace("19444", "0"), headers("html")["content-security-policy"] + "; script-src *"):
        with pytest.raises(AssertionError):
            validate_gateway("html", expected_artifact("html"), {**headers("html"), "content-security-policy": policy}, "/tmp/service.sock")
