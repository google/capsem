"""Real confined gateway -> granted FD -> service -> synthetic Rust OTLP owner.

Requires one exact host cohort including capsem-mock-server. The mock's Rust
production admission/generated protobuf decoder is the oracle; Python only
reads its normalized metadata summary. No real collector/provider credentials.
"""
import http.client
import json
import os
import time
from urllib.parse import urlsplit

import pytest
from helpers.mock_server import start_mock_server, stop_process
from helpers.service import ServiceInstance
from log_streams import read_log_stream

pytestmark = pytest.mark.integration
AUTH = "Bearer capsem-test-collector"
FAKE_VM = "00000000-0000-4000-8000-000000000000"
CANARIES = ("private-path-canary", "PRIVATE-METHOD-CANARY", "query-canary", "body-canary", "header-canary", "foreign-host-canary", "capsem-test-collector", FAKE_VM)
MAX_SUMMARY_BYTES = 4 * 1024 * 1024


def _http(port, method, path, headers=None, body=b""):
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=10)
    try:
        connection.request(method, path, body=body, headers=headers or {})
        response = connection.getresponse()
        payload = response.read(MAX_SUMMARY_BYTES + 1)
        assert len(payload) <= MAX_SUMMARY_BYTES, "bounded fixture response exceeded"
        return response.status, payload
    finally:
        connection.close()


@pytest.fixture
def collector(tmp_path):
    log = tmp_path / "collector.jsonl"
    process, ready = start_mock_server(addr="127.0.0.1:0", request_log=log, otlp_http_fixture=True)
    try:
        assert "/otlp/v1/metrics" in ready["endpoints"]
        yield ready, log
    finally:
        stop_process(process)


def _clear_otel(monkeypatch):
    # Names only: never inspect, print or replay any ambient credential value.
    for name in tuple(os.environ):
        if name.startswith("OTEL_"):
            monkeypatch.delenv(name)


def _summary(ready):
    port = urlsplit(ready["base_url"]).port
    status, body = _http(port, "GET", "/otlp/summary", {"Authorization": AUTH})
    assert status == 200
    result = json.loads(body)
    assert len(result["points"]) <= 4096
    assert not any(value.encode() in body for value in CANARIES)
    assert AUTH.encode() not in body
    return result


def _count(summary, name, **labels):
    return sum(point["value"] for point in summary["points"]
               if point["name"] == name and all(point["labels"].get(key) == value for key, value in labels.items()))


def _wait_summary(ready, predicate):
    # The confined child does not inherit collector/OTEL configuration. Its
    # normal SDK periodic reader remains at the standard 60-second cadence.
    deadline = time.monotonic() + 90
    while time.monotonic() < deadline:
        value = _summary(ready)
        if predicate(value):
            return value
        time.sleep(0.1)
    raise AssertionError("bounded native gateway metric export did not arrive")


def _gateway(service):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        try:
            port = int((service.tmp_dir / "gateway.port").read_text())
            token = (service.tmp_dir / "gateway.token").read_text().strip()
            if token:
                return port, {"Authorization": f"Bearer {token}"}
        except (FileNotFoundError, ValueError):
            pass
        time.sleep(0.05)
    raise AssertionError("native gateway private readiness did not arrive")


def _private_logs(service):
    service.stop(cleanup=False)  # Flush both owned tracing workers first.
    logs = read_log_stream(service.tmp_dir / "service.log")
    gateway = read_log_stream(service.tmp_dir / "logs/gateway.log")
    assert logs and gateway, "privacy assertion requires actual runtime logs"
    combined = logs + gateway
    assert not any(value in combined for value in CANARIES)
    return combined


def test_gateway_metric_export_preserves_normalized_facts_and_restarts_without_old_series(collector, monkeypatch):
    ready, log = collector
    _clear_otel(monkeypatch)
    monkeypatch.setenv("OTEL_EXPORTER_OTLP_ENDPOINT", ready["base_url"] + "/otlp")
    monkeypatch.setenv("OTEL_EXPORTER_OTLP_HEADERS", "Authorization=Bearer%20capsem-test-collector")
    monkeypatch.setenv("RUST_LOG", "capsem_gateway=debug,capsem_telemetry=debug,capsem_service=info")
    service = ServiceInstance(isolated_corp_config=True)
    try:
        service.start()
        port, headers = _gateway(service)
        assert _http(port, "GET", "/health")[0] == 200
        assert _http(port, "GET", f"/vms/{FAKE_VM}/info?token=query-canary", {"Authorization": "Bearer header-canary"})[0] == 401
        assert _http(port, "GET", "/health", {"Host": "foreign-host-canary.invalid"})[0] == 403
        for index in range(100):
            assert _http(port, f"PRIVATE-METHOD-CANARY-{index}", f"/private-path-canary-{index}?secret=query-canary", headers, b"body-canary")[0] == 404
        assert _http(port, "OPTIONS", "/health", {"Origin": "http://localhost:19222", "Access-Control-Request-Method": "GET"})[0] == 200
        assert _http(port, "GET", "/status", headers)[0] == 200
        first = _wait_summary(ready, lambda value: _count(value, "http.server.requests_total", method="OTHER", route="unmatched") == 100)
        assert first["authorization_verified"] and first["wire_bytes"] > 0
        assert _count(first, "http.server.responses_total", status="401", route="/vms/{id}/info") == 1
        assert _count(first, "http.server.responses_total", status="403", route="/health") == 1
        assert _count(first, "http.server.responses_total", status="404", route="unmatched") == 100
        assert _count(first, "http.server.response_ready_ms", route="/status") >= 1
        assert _count(first, "http.server.body_lifetime_ms", route="/status", completion="eof") >= 1
        assert len(first["points"]) < 100, "100 private inputs must not grow label cardinality"
        _private_logs(service)
        before_restart = _summary(ready)["batches"]
        service.start()
        port, _ = _gateway(service)
        for _ in range(3):
            assert _http(port, "GET", "/health")[0] == 200
        second = _wait_summary(ready, lambda value: value["batches"] > before_restart
                               and _count(value, "http.server.requests_total", method="GET", route="/health") >= 3
                               and _count(value, "http.server.requests_total", method="OTHER", route="unmatched") == 0)
        assert second["authorization_verified"]
        _private_logs(service)
        assert log.read_bytes() == b"", "collector payload/header logging is forbidden"
    finally:
        service.stop()


def test_gateway_optional_export_off_keeps_normal_service_access_without_collector_traffic(collector, monkeypatch):
    ready, log = collector
    _clear_otel(monkeypatch)
    service = ServiceInstance(isolated_corp_config=True)
    try:
        service.start()
        port, headers = _gateway(service)
        assert _http(port, "GET", "/health")[0] == 200
        assert _http(port, "GET", "/status", headers)[0] == 200
        assert service.client().get("/vms/list")["sandboxes"] == []
        _private_logs(service)
        assert _summary(ready)["batches"] == 0
        assert log.read_bytes() == b""
    finally:
        service.stop()
