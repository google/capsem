import sys
import types
import json
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).parent.parent
ARTIFACTS = PROJECT_ROOT / "guest" / "artifacts"
sys.path.insert(0, str(ARTIFACTS))


class _StubConsole:
    def __init__(self, *args, **kwargs):
        pass

    def print(self, *args, **kwargs):
        pass


class _StubTable:
    def __init__(self, *args, **kwargs):
        self.rows = []

    def add_column(self, *args, **kwargs):
        pass

    def add_row(self, *args, **kwargs):
        self.rows.append(args)


rich_module = types.ModuleType("rich")
rich_console = types.ModuleType("rich.console")
rich_table = types.ModuleType("rich.table")
rich_text = types.ModuleType("rich.text")
rich_console.Console = _StubConsole
rich_table.Table = _StubTable
rich_text.Text = str
sys.modules.setdefault("rich", rich_module)
sys.modules.setdefault("rich.console", rich_console)
sys.modules.setdefault("rich.table", rich_table)
sys.modules.setdefault("rich.text", rich_text)

from capsem_bench import __main__ as bench_main  # noqa: E402
from capsem_bench import http_bench, throughput  # noqa: E402
from capsem_bench import mitm_local  # noqa: E402
from capsem_bench import load_harness  # noqa: E402
from helpers.debug_upstream import start_debug_upstream, stop_process  # noqa: E402


def test_mitm_local_is_not_a_top_level_escape_hatch():
    assert "mitm-local" not in bench_main.VALID_MODES
    assert "storage" in bench_main.VALID_MODES
    assert "all" in bench_main.VALID_MODES


def test_all_mode_includes_local_mitm_when_debug_upstream_is_configured(monkeypatch):
    monkeypatch.setenv(mitm_local.BASE_URL_ENV, "http://127.0.0.1:3713")

    assert bench_main._should_run_local_mitm("all") is True
    assert bench_main._should_run_local_mitm("disk") is False


def test_http_bench_default_skips_without_local_or_public(monkeypatch):
    monkeypatch.delenv(http_bench.LOCAL_DEBUG_UPSTREAM_ENV, raising=False)
    monkeypatch.delenv("CAPSEM_BENCH_ALLOW_PUBLIC_NETWORK", raising=False)
    result = http_bench.http_bench()
    assert result["skipped"] is True
    assert "local lab" in result["reason"]


def test_http_bench_prefers_local_debug_upstream(monkeypatch):
    monkeypatch.setenv(http_bench.LOCAL_DEBUG_UPSTREAM_ENV, "http://127.0.0.1:1234/")
    monkeypatch.delenv("CAPSEM_BENCH_ALLOW_PUBLIC_NETWORK", raising=False)
    assert http_bench._default_http_url() == "http://127.0.0.1:1234/tiny"


def test_throughput_default_skips_without_local_or_public(monkeypatch):
    monkeypatch.delenv(throughput.LOCAL_DEBUG_UPSTREAM_ENV, raising=False)
    monkeypatch.delenv("CAPSEM_BENCH_ALLOW_PUBLIC_NETWORK", raising=False)
    result = throughput.throughput_bench()
    assert result["skipped"] is True
    assert "local lab" in result["reason"]


def test_throughput_prefers_local_debug_upstream(monkeypatch):
    monkeypatch.setenv(throughput.LOCAL_DEBUG_UPSTREAM_ENV, "http://127.0.0.1:1234/")
    monkeypatch.delenv("CAPSEM_BENCH_ALLOW_PUBLIC_NETWORK", raising=False)
    target = throughput._throughput_target()
    assert target == (
        "http://127.0.0.1:1234/bytes/10mb",
        throughput.LOCAL_THROUGHPUT_EXPECTED_BYTES,
        "local",
    )


def test_base_url_requires_explicit_local_upstream(monkeypatch):
    monkeypatch.delenv(mitm_local.BASE_URL_ENV, raising=False)
    with pytest.raises(ValueError, match=mitm_local.BASE_URL_ENV):
        mitm_local._base_url(None)


def test_base_url_accepts_env_and_strips_trailing_slash(monkeypatch):
    monkeypatch.setenv(mitm_local.BASE_URL_ENV, "http://127.0.0.1:1234/")
    assert mitm_local._base_url(None) == "http://127.0.0.1:1234"


def test_base_url_rejects_non_http():
    with pytest.raises(ValueError, match="invalid mitm-local base URL"):
        mitm_local._base_url("file:///tmp/debug-upstream")


def test_ws_url_matches_base_scheme():
    assert (
        mitm_local._ws_url("http://127.0.0.1:1234", "/ws/echo")
        == "ws://127.0.0.1:1234/ws/echo"
    )
    assert (
        mitm_local._ws_url("https://example.test", "/ws/echo")
        == "wss://example.test/ws/echo"
    )


def test_websocket_uses_plain_url_without_socket_override(monkeypatch):
    captured = {}

    class FakeWebSocket:
        def __init__(self):
            self.last_payload = None

        def __enter__(self):
            return self

        def __exit__(self, *_args):
            return False

        def send(self, payload):
            self.last_payload = payload

        def recv(self, timeout=None):
            return self.last_payload

    def fake_connect(url, **kwargs):
        captured["url"] = url
        captured["connect_kwargs"] = kwargs
        return FakeWebSocket()

    import websockets.sync.client as ws_client

    monkeypatch.setattr(ws_client, "connect", fake_connect)

    result = mitm_local._run_websocket_scenario(
        "http://127.0.0.1:50233",
        {"name": "websocket_echo", "path": "/ws/echo", "frames": 1},
        timeout_s=5,
    )

    assert result["failed"] is False
    assert captured["url"] == "ws://127.0.0.1:50233/ws/echo"
    assert "sock" not in captured["connect_kwargs"]
    assert captured["connect_kwargs"]["proxy"] is None


def test_http_summary_has_latency_and_no_raw_secret_storage():
    scenario = {
        "name": "credential_response",
        "path": "/credential/response",
        "expected_status": 200,
        "body_kind": "credential",
        "secret_shaped_fixture": True,
    }
    results = [
        {
            "status": 200,
            "size": 128,
            "latency_ms": 1.0,
            "error": None,
            "required_text_present": True,
            "secret_shaped_fixture_seen": True,
        },
        {
            "status": 200,
            "size": 128,
            "latency_ms": 5.0,
            "error": None,
            "required_text_present": True,
            "secret_shaped_fixture_seen": True,
        },
    ]
    summary = mitm_local._summarize_http_results(
        scenario, results, wall_time_s=0.01, total_requests=2, concurrency=1
    )
    assert summary["successful"] == 2
    assert summary["failed"] == 0
    assert summary["latency_ms"]["p50"] == 3.0
    assert summary["secret_shaped_fixture_seen"] is True
    assert summary["raw_secret_stored_in_result"] is False
    assert "capsem_test_" not in repr(summary)


def test_env_defaults_are_fast_and_overrideable(monkeypatch):
    calls = []

    def fake_http(base_url, scenario, total_requests, concurrency, timeout_s):
        calls.append((scenario["name"], total_requests, concurrency, timeout_s))
        return {
            "name": scenario["name"],
            "path": scenario["path"],
            "body_kind": scenario["body_kind"],
            "total_requests": total_requests,
            "concurrency": concurrency,
            "successful": total_requests,
            "failed": 0,
            "total_duration_ms": 1.0,
            "requests_per_sec": 1000.0,
            "transfer_bytes": 1,
            "bytes_per_sec": 1000.0,
            "latency_ms": {
                "min": 1.0,
                "max": 1.0,
                "mean": 1.0,
                "p50": 1.0,
                "p95": 1.0,
                "p99": 1.0,
            },
            "errors": {},
        }

    monkeypatch.setenv(mitm_local.BASE_URL_ENV, "http://127.0.0.1:9999")
    monkeypatch.setenv(load_harness.GLOBAL_TOTAL_REQUESTS_ENV, "3")
    monkeypatch.setenv(load_harness.GLOBAL_CONCURRENCY_ENV, "2")
    monkeypatch.setenv(load_harness.GLOBAL_TIMEOUT_ENV, "4")
    monkeypatch.setattr(mitm_local, "_run_http_scenario", fake_http)
    monkeypatch.setattr(mitm_local, "_run_websocket_scenario", lambda *_: {
        "name": "websocket_echo",
        "path": "/ws/echo",
        "skipped": True,
        "frames": 0,
        "frames_per_sec": 0.0,
        "latency_ms": {
            "min": 0.0,
            "max": 0.0,
            "mean": 0.0,
            "p50": 0.0,
            "p95": 0.0,
            "p99": 0.0,
        },
    })

    result = mitm_local.mitm_local_bench()

    assert result["base_url"] == "http://127.0.0.1:9999"
    assert result["total_requests"] == 3
    assert result["concurrency"] == 2
    assert result["timeout_s"] == 4.0
    assert len(result["scenarios"]) == len(mitm_local.HTTP_SCENARIOS)
    assert calls[0] == ("tiny_http", 3, 2, 4.0)


def test_local_mitm_defaults_are_release_grade():
    assert mitm_local.DEFAULT_TOTAL_REQUESTS >= 50_000
    assert mitm_local.DEFAULT_CONCURRENCY >= 64


def test_global_load_config_parses_count_and_duration_modes(monkeypatch):
    monkeypatch.setenv(load_harness.GLOBAL_CONCURRENCY_ENV, "64")
    monkeypatch.setenv(load_harness.GLOBAL_DURATION_ENV, "7.5")
    duration = load_harness.DurationLoadConfig.from_inputs(
        "dns-load",
        default_concurrency=(1, 10),
        default_duration_s=10,
    )
    assert duration.concurrency_levels == (64,)
    assert duration.duration_s == 7.5

    monkeypatch.setenv(load_harness.GLOBAL_TOTAL_REQUESTS_ENV, "50000")
    monkeypatch.setenv(load_harness.GLOBAL_TIMEOUT_ENV, "9")
    monkeypatch.setenv(load_harness.GLOBAL_SCENARIOS_ENV, "model_json_response")
    count = load_harness.CountLoadConfig.from_inputs(
        "mitm-local",
        default_total_requests=20,
        default_concurrency=1,
        default_timeout_s=30,
    )
    assert count.total_requests == 50_000
    assert count.concurrency == 64
    assert count.timeout_s == 9.0
    assert count.scenarios == ("model_json_response",)


def test_mode_specific_load_config_overrides_global(monkeypatch):
    monkeypatch.setenv(load_harness.GLOBAL_CONCURRENCY_ENV, "64")
    monkeypatch.setenv("CAPSEM_BENCH_DNS_LOAD_CONCURRENCY", "1,32")
    config = load_harness.DurationLoadConfig.from_inputs(
        "dns-load",
        default_concurrency=(1, 10),
        default_duration_s=10,
    )
    assert config.concurrency_levels == (1, 32)


@pytest.mark.parametrize("value", ["", "0", "-1", "one"])
def test_load_config_rejects_bad_concurrency(value):
    with pytest.raises(ValueError):
        load_harness.parse_concurrency_levels(value)


def test_scenario_selection_filters_http_scenarios(monkeypatch):
    calls = []

    def fake_http(base_url, scenario, total_requests, concurrency, timeout_s):
        calls.append((scenario["name"], total_requests, concurrency, timeout_s))
        return {
            "name": scenario["name"],
            "path": scenario["path"],
            "body_kind": scenario["body_kind"],
            "total_requests": total_requests,
            "concurrency": concurrency,
            "successful": total_requests,
            "failed": 0,
            "total_duration_ms": 1.0,
            "requests_per_sec": 1000.0,
            "transfer_bytes": 1,
            "bytes_per_sec": 1000.0,
            "latency_ms": {
                "min": 1.0,
                "max": 1.0,
                "mean": 1.0,
                "p50": 1.0,
                "p95": 1.0,
                "p99": 1.0,
            },
            "errors": {},
        }

    monkeypatch.setattr(mitm_local, "_run_http_scenario", fake_http)
    monkeypatch.setattr(mitm_local, "_run_websocket_scenario", lambda *_: {
        "name": "websocket_echo",
        "path": "/ws/echo",
        "skipped": True,
        "frames": 0,
        "frames_per_sec": 0.0,
        "latency_ms": {
            "min": 0.0,
            "max": 0.0,
            "mean": 0.0,
            "p50": 0.0,
            "p95": 0.0,
            "p99": 0.0,
        },
    })

    result = mitm_local.mitm_local_bench(
        base_url="http://127.0.0.1:9999",
        total_requests=50_000,
        concurrency=64,
        timeout_s=4,
        scenarios="model_json_response,credential_response",
    )

    assert result["selected_scenarios"] == [
        "model_json_response",
        "credential_response",
    ]
    assert [call[0] for call in calls] == [
        "model_json_response",
        "credential_response",
    ]
    assert all(call[1] == 50_000 for call in calls)
    assert all(call[2] == 64 for call in calls)


def test_scenario_selection_rejects_unknown_name():
    with pytest.raises(ValueError, match="unknown mitm-local scenario"):
        mitm_local.mitm_local_bench(
            base_url="http://127.0.0.1:9999",
            scenarios="model_json_response,not_real",
        )


def test_mitm_local_drives_debug_http_fixture():
    proc = None
    try:
        proc, ready = start_debug_upstream()
        result = mitm_local.mitm_local_bench(
            base_url=ready["base_url"],
            total_requests=1,
            concurrency=1,
            timeout_s=5,
        )
    finally:
        stop_process(proc)

    by_name = {row["name"]: row for row in result["scenarios"]}
    assert by_name["tiny_http"]["successful"] == 1
    assert by_name["http_1mb"]["successful"] == 1
    assert by_name["gzip_1mb"]["successful"] == 1
    assert by_name["sse_model"]["successful"] == 1
    assert by_name["model_json_response"]["successful"] == 1
    assert by_name["denied_target"]["successful"] == 1
    assert by_name["credential_response"]["successful"] == 1
    assert by_name["credential_response"]["secret_shaped_fixture_seen"] is True
    assert "capsem_test_api_key" not in json.dumps(result)
