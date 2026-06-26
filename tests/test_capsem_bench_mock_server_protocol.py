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
        self.messages = []

    def print(self, *args, **kwargs):
        self.messages.append(" ".join(str(arg) for arg in args))


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
from capsem_bench import load_harness  # noqa: E402


def test_python_capsem_bench_has_no_http_protocol_or_throughput_modes():
    assert "http" not in bench_main.VALID_MODES
    assert "throughput" not in bench_main.VALID_MODES
    assert "protocol" not in bench_main.VALID_MODES
    assert "storage" in bench_main.VALID_MODES
    assert "all" in bench_main.VALID_MODES


@pytest.mark.parametrize("mode", ["http", "throughput"])
def test_python_hot_bench_modes_fail_closed_and_point_to_rust(mode, monkeypatch):
    monkeypatch.setattr(sys, "argv", ["capsem-bench", mode])
    console = _StubConsole()
    monkeypatch.setattr(bench_main, "console", console)

    with pytest.raises(SystemExit) as exc:
        bench_main.main()

    assert exc.value.code == 127
    output = "\n".join(console.messages)
    assert f"capsem-bench {mode} is retired from Python" in output
    assert "capsem-bench-rs" in output


def test_protocol_mode_delegates_to_rust_bench(monkeypatch):
    calls = []

    class Completed:
        returncode = 7

    monkeypatch.setattr(
        sys,
        "argv",
        ["capsem-bench", "protocol", "--base-url", "http://127.0.0.1:3713"],
    )
    monkeypatch.setattr(bench_main.os.path, "exists", lambda path: path == bench_main.RUST_BENCH)
    monkeypatch.setattr(
        bench_main.subprocess,
        "run",
        lambda argv, check=False: calls.append((argv, check)) or Completed(),
    )

    with pytest.raises(SystemExit) as exc:
        bench_main.main()

    assert exc.value.code == 7
    assert calls == [
        ([
            bench_main.RUST_BENCH,
            "protocol",
            "--base-url",
            "http://127.0.0.1:3713",
        ], False)
    ]


def test_all_mode_keeps_network_sections_by_merging_rust_protocol(monkeypatch):
    def fake_module(module_name, function_name):
        module = types.ModuleType(module_name)
        setattr(module, function_name, lambda: {"ok": module_name})
        monkeypatch.setitem(sys.modules, module_name, module)

    fake_module("capsem_bench.disk", "disk_bench")
    fake_module("capsem_bench.rootfs", "rootfs_bench")
    fake_module("capsem_bench.storage", "storage_bench")
    fake_module("capsem_bench.startup", "startup_bench")
    fake_module("capsem_bench.snapshot", "snapshot_bench")
    monkeypatch.setattr(sys, "argv", ["capsem-bench", "all"])
    monkeypatch.setenv(bench_main.MOCK_SERVER_PROTOCOL_BASE_URL_ENV, "http://127.0.0.1:3713")
    calls = []

    def fake_rust_protocol_artifact(scenarios=None, requests=None, concurrency=None):
        calls.append((scenarios, requests, concurrency))
        rows = []
        if scenarios is None:
            rows.append(
                {
                    "name": "model_json_response",
                    "path": "/model/response",
                    "total_requests": 50,
                    "successful": 50,
                    "failed": 0,
                    "requests_per_sec": 1000.0,
                    "latency_ms": {"p99": 1.0},
                    "bytes_per_sec": 24000.0,
                    "transfer_bytes": 1200,
                }
            )
        elif scenarios == "tiny_http":
            rows.append(
                {
                    "name": "tiny_http",
                    "path": "/tiny",
                    "total_requests": 50,
                    "successful": 50,
                    "failed": 0,
                    "requests_per_sec": 1000.0,
                    "latency_ms": {"p99": 1.0},
                    "bytes_per_sec": 24000.0,
                    "transfer_bytes": 1200,
                }
            )
        elif scenarios == "http_10mb":
            rows.append(
                {
                    "name": "http_10mb",
                    "path": "/bytes/10mb",
                    "total_requests": 1,
                    "successful": 1,
                    "failed": 0,
                    "requests_per_sec": 1.0,
                    "latency_ms": {"p99": 10.0},
                    "bytes_per_sec": 10 * 1024 * 1024,
                    "transfer_bytes": 10 * 1024 * 1024,
                    "total_duration_ms": 1000.0,
                }
            )
        return {
            "mock_server_protocol": {
                "base_url": "http://127.0.0.1:3713",
                "scenarios": rows,
            }
        }

    monkeypatch.setattr(bench_main, "_run_rust_protocol_artifact", fake_rust_protocol_artifact)

    bench_main.main()

    data = json.loads(Path("/tmp/capsem-benchmark.json").read_text())
    assert data["http"]["total_requests"] == 50
    assert data["http"]["failed"] == 0
    assert data["throughput"]["http_code"] == 200
    assert data["throughput"]["size_bytes"] == 10 * 1024 * 1024
    assert "mock_server_protocol" in data
    assert calls == [(None, None, None), ("tiny_http", None, None), ("http_10mb", "1", "1")]


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
        "capsem-bench-rs protocol",
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
