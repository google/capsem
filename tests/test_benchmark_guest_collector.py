"""The shared guest collector preserves hermetic controls and diagnostics."""

from __future__ import annotations

import json
import os
import runpy
import shlex
import sys
from pathlib import Path
from types import SimpleNamespace

import pytest
from capsem_builder.gate import config as gate_config

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)
COLLECTOR = CONFIG.path(CONFIG.benchmark.run.collectors) / "_guest"


def collector() -> dict:
    return runpy.run_path(str(COLLECTOR))


class ResourceSamplerStub:
    def start(self) -> None:
        pass

    def stop(self) -> dict:
        return {}


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


def test_storage_controls_allow_a_focused_workspace_measurement(monkeypatch) -> None:
    controls = {
        "CAPSEM_STORAGE_BENCH_PATHS": "/root",
        "CAPSEM_STORAGE_BENCH_SIZE_MB": "8",
        "CAPSEM_STORAGE_IO_PROFILE_SIZE_MB": "8",
        "CAPSEM_STORAGE_IO_PROFILE_RANDOM_OPS": "16",
    }
    for name, value in controls.items():
        monkeypatch.setenv(name, value)
    args = shlex.split(collector()["guest_command"]("storage"))
    for name, value in controls.items():
        assert f"{name}={value}" in args


def test_protocol_command_forwards_fixture_controls_and_requests_raw_samples(
    monkeypatch,
) -> None:
    monkeypatch.setenv("CAPSEM_MOCK_SERVER_BASE_URL", "http://127.0.0.1:3713")
    monkeypatch.setenv("CAPSEM_MOCK_SERVER_DNS_UDP_ADDR", "127.0.0.1:41000")
    monkeypatch.setenv("CAPSEM_BENCH_TOTAL_REQUESTS", "100")
    monkeypatch.setenv("CAPSEM_BENCH_SCENARIOS", "model_json_response,mcp_tool_call")

    args = shlex.split(collector()["guest_command"]("protocol"))
    assert "CAPSEM_MOCK_SERVER_BASE_URL=http://127.0.0.1:3713" in args
    assert "CAPSEM_MOCK_SERVER_DNS_UDP_ADDR=127.0.0.1:41000" in args
    assert "CAPSEM_BENCH_TOTAL_REQUESTS=100" in args
    assert "CAPSEM_BENCH_SCENARIOS=model_json_response,mcp_tool_call" in args
    assert args[-5:] == [
        "protocol",
        "--lane",
        "guest_capsem",
        "--collector-out",
        "/tmp/capsem-benchmark-collector.json",
    ]


def test_protocol_defaults_come_from_the_typed_gate_config(monkeypatch) -> None:
    controls = (
        "CAPSEM_BENCH_TOTAL_REQUESTS",
        "CAPSEM_BENCH_CONCURRENCY",
        "CAPSEM_BENCH_TIMEOUT_MS",
        "CAPSEM_BENCH_SCENARIOS",
    )
    for name in controls:
        monkeypatch.delenv(name, raising=False)

    module = collector()
    module["apply_protocol_defaults"]()
    settings = CONFIG.benchmark.protocol

    assert os.environ["CAPSEM_BENCH_TOTAL_REQUESTS"] == str(settings.requests)
    assert os.environ["CAPSEM_BENCH_CONCURRENCY"] == str(settings.concurrency)
    assert os.environ["CAPSEM_BENCH_TIMEOUT_MS"] == str(settings.timeout_ms)
    assert os.environ["CAPSEM_BENCH_SCENARIOS"] == ",".join(settings.scenarios)
    assert "dns_local_nxdomain" not in settings.scenarios


def test_mitm_scale_settings_cover_independent_vm_fanout() -> None:
    settings = CONFIG.benchmark.mitm_scale
    assert tuple(settings.vm_counts) == (1, 2, 4, 8)
    assert settings.per_vm_concurrency == 16
    assert settings.duration_seconds == 10


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


def test_mitm_fixture_corp_config_routes_dns_and_the_fixture_host_to_the_mock_server() -> (
    None
):
    module = collector()
    toml = module["mitm_fixture_corp_toml"](
        {"dns_udp_addr": "127.0.0.1:41000", "http_addr": "127.0.0.1:3713"}
    )
    assert 'upstreams = ["127.0.0.1:41000"]' in toml
    assert '[network.upstream_overrides."fixture.capsem.test:443"]' in toml
    assert 'dial = "127.0.0.1:3713"' in toml
    assert 'protocol = "http"' in toml
    assert module["MITM_FIXTURE_TARGET"] == "https://fixture.capsem.test/tiny"


def test_a_level_where_any_request_failed_is_not_a_measurement() -> None:
    failed = collector()["failed_load_levels"]
    section = {
        "concurrency_levels": [
            {"concurrency": 1, "total_requests": 700, "errors": 700},
            {"concurrency": 10, "total_requests": 6000, "errors": 12},
            {"concurrency": 50, "total_requests": 0, "errors": 0},
            {"concurrency": 200, "total_requests": 100, "errors": 250},
        ]
    }
    assert failed(section) == [1, 10, 200]
    assert failed({"concurrency_levels": []}) == []
    assert failed({}) == []


def test_mitm_runtime_refusals_are_fatal_even_when_the_client_retries() -> None:
    failures = collector()["mitm_runtime_failures"]
    log = (
        '{"fields":{"message":"service.start"}}\n'
        '{"fields":{"message":"proxy worker refused traffic descriptor",'
        '"error":"proxy worker rejected HttpTraffic grant 522: Capacity"}}\n'
        '{"fields":{"message":"proxy worker refused traffic descriptor",'
        '"error":"proxy worker rejected HttpTraffic grant 523: Capacity"}}'
    )

    assert failures(log) == [
        "proxy worker rejected HttpTraffic grant 522: Capacity",
        "proxy worker rejected HttpTraffic grant 523: Capacity",
    ]
    assert failures('{"fields":{"message":"service.start"}}') == []


def test_guest_result_rejects_a_hidden_proxy_refusal(monkeypatch, tmp_path) -> None:
    document = {
        "mitm_load": {
            "concurrency_levels": [
                {"concurrency": 512, "total_requests": 600, "errors": 0, "rps": 60.0},
            ]
        }
    }
    client = SimpleNamespace(
        post=lambda route, *args, **kwargs: (
            {"id": "bench-test-vm"}
            if route == "/vms/create"
            else {
                "exit_code": 0,
                "stdout": {"encoding": "utf8", "data": json.dumps(document)},
            }
        ),
        delete=lambda *args: None,
    )
    log = (
        '{"fields":{"message":"proxy worker refused traffic descriptor",'
        '"error":"proxy worker rejected HttpTraffic grant 522: Capacity"}}'
    )
    main = collector()["main"]
    monkeypatch.setitem(
        main.__globals__,
        "ServiceInstance",
        lambda: SimpleNamespace(
            client=lambda: client,
            start=lambda: None,
            stop=lambda: None,
            stop_and_read_log=lambda: log,
            tmp_dir=tmp_path,
        ),
    )
    import helpers.mock_server

    monkeypatch.setattr(
        helpers.mock_server,
        "start_mock_server",
        lambda: (
            object(),
            {"dns_udp_addr": "127.0.0.1:41000", "http_addr": "127.0.0.1:3713"},
        ),
    )
    monkeypatch.setattr(helpers.mock_server, "stop_process", lambda proc: None)
    monkeypatch.setitem(
        main.__globals__, "wait_exec_ready", lambda *args, **kwargs: True
    )
    monkeypatch.setitem(
        main.__globals__, "process_tree_sampler", lambda service: ResourceSamplerStub()
    )
    monkeypatch.setattr(sys, "argv", ["mitm-load"])

    with pytest.raises(RuntimeError, match="1 internal traffic descriptor refusal"):
        main()


def test_load_dimensions_get_more_guest_cpus_bounded_by_the_host() -> None:
    guest_cpus = collector()["guest_cpus"]
    assert guest_cpus("disk", host_cpus=16) == 2, "non-load modes keep the default"
    assert guest_cpus("mitm-load", host_cpus=16) == 8
    assert guest_cpus("dns-load", host_cpus=64) == 8, "capped"
    assert guest_cpus("mcp-load", host_cpus=6) == 3
    assert guest_cpus("protocol", host_cpus=16) == 8
    assert guest_cpus("mitm-load", host_cpus=2) == 2, "never below the default"


def test_guest_result_preserves_zero_error_proof(monkeypatch, capsys) -> None:
    document = {
        "dns_load": {
            "concurrency_levels": [
                {"concurrency": 200, "total_requests": 500, "errors": 0, "rps": 50.0},
            ]
        }
    }
    stopped = []
    client = SimpleNamespace(
        post=lambda route, *args, **kwargs: (
            {"id": "bench-test-vm"}
            if route == "/vms/create"
            else {
                "exit_code": 0,
                "stdout": {"encoding": "utf8", "data": json.dumps(document)},
            }
        ),
        delete=lambda *args: None,
    )
    main = collector()["main"]
    monkeypatch.setitem(
        main.__globals__,
        "ServiceInstance",
        lambda: SimpleNamespace(
            client=lambda: client,
            start=lambda: None,
            stop=lambda: stopped.append(True),
        ),
    )
    monkeypatch.setitem(
        main.__globals__, "wait_exec_ready", lambda *args, **kwargs: True
    )
    monkeypatch.setitem(
        main.__globals__, "process_tree_sampler", lambda service: ResourceSamplerStub()
    )
    monkeypatch.setattr(sys, "argv", ["dns-load"])

    assert main() == 0
    result = json.loads(capsys.readouterr().out)
    assert json.loads(result["sidecar"]) == document
    assert result["metrics"] == {
        "concurrency_levels.0.error_ratio": {"unit": "ratio", "samples": [0.0]},
        "concurrency_levels.0.errors": {"unit": "count", "samples": [0]},
        "concurrency_levels.0.requests": {"unit": "count", "samples": [500]},
        "concurrency_levels.0.rps": {"unit": "requests_per_second", "samples": [50.0]},
    }
    assert stopped == [True]


def test_protocol_result_preserves_raw_model_samples(monkeypatch, capsys) -> None:
    artifact = {
        "mock_server_protocol": {
            "total_requests": 3,
            "scenarios": [
                {
                    "name": "model_json_response",
                    "successful": 3,
                    "failed": 0,
                    "requests_per_sec": 750.0,
                }
            ],
        }
    }
    collection = {
        "metrics": {
            "guest_capsem.model_json_response.latency_ms": {
                "unit": "milliseconds",
                "samples": [0.5, 1.5, 2.5],
            }
        },
        "sidecar": json.dumps(artifact),
    }

    def post(route, body=None, **kwargs):
        if route == "/vms/create":
            return {"id": "bench-test-vm"}
        assert body is not None
        output = (
            json.dumps(collection)
            if body["command"] == "cat /tmp/capsem-benchmark-collector.json"
            else "benchmark output"
        )
        return {"exit_code": 0, "stdout": {"encoding": "utf8", "data": output}}

    client = SimpleNamespace(post=post, delete=lambda *args: None)
    main = collector()["main"]
    monkeypatch.setitem(
        main.__globals__,
        "ServiceInstance",
        lambda: SimpleNamespace(
            client=lambda: client,
            start=lambda: None,
            stop=lambda: None,
            stop_and_read_log=lambda: "",
        ),
    )
    import helpers.mock_server

    monkeypatch.setattr(
        helpers.mock_server,
        "start_mock_server",
        lambda **kwargs: (
            object(),
            {
                "base_url": "http://127.0.0.1:3713",
                "dns_udp_addr": "127.0.0.1:41000",
            },
        ),
    )
    monkeypatch.setattr(helpers.mock_server, "stop_process", lambda proc: None)
    monkeypatch.setitem(
        main.__globals__, "wait_exec_ready", lambda *args, **kwargs: True
    )
    monkeypatch.setitem(
        main.__globals__, "process_tree_sampler", lambda service: ResourceSamplerStub()
    )
    monkeypatch.setattr(sys, "argv", ["protocol"])

    assert main() == 0
    result = json.loads(capsys.readouterr().out)
    assert result["metrics"]["guest_capsem.model_json_response.latency_ms"][
        "samples"
    ] == [0.5, 1.5, 2.5]
    assert json.loads(result["sidecar"]) == artifact


def test_guest_result_rejects_one_failed_request(monkeypatch, tmp_path) -> None:
    document = {
        "mitm_load": {
            "concurrency_levels": [
                {"concurrency": 200, "total_requests": 500, "errors": 1, "rps": 49.9},
            ]
        }
    }
    client = SimpleNamespace(
        post=lambda route, *args, **kwargs: (
            {"id": "bench-test-vm"}
            if route == "/vms/create"
            else {
                "exit_code": 0,
                "stdout": {"encoding": "utf8", "data": json.dumps(document)},
            }
        ),
        delete=lambda *args: None,
    )
    main = collector()["main"]
    monkeypatch.setitem(
        main.__globals__,
        "ServiceInstance",
        lambda: SimpleNamespace(
            client=lambda: client,
            start=lambda: None,
            stop=lambda: None,
            stop_and_read_log=lambda: "",
            tmp_dir=tmp_path,
        ),
    )
    import helpers.mock_server

    monkeypatch.setattr(
        helpers.mock_server,
        "start_mock_server",
        lambda: (
            object(),
            {"dns_udp_addr": "127.0.0.1:41000", "http_addr": "127.0.0.1:3713"},
        ),
    )
    monkeypatch.setattr(helpers.mock_server, "stop_process", lambda proc: None)
    monkeypatch.setitem(
        main.__globals__, "wait_exec_ready", lambda *args, **kwargs: True
    )

    class FailedRunSampler(ResourceSamplerStub):
        def stop(self) -> dict:
            return {
                "resources.total.cpu_seconds": {
                    "unit": "seconds",
                    "samples": [12.5],
                },
                "resources.total.peak_rss_bytes": {
                    "unit": "bytes",
                    "samples": [1048576],
                },
            }

    monkeypatch.setitem(
        main.__globals__, "process_tree_sampler", lambda service: FailedRunSampler()
    )
    monkeypatch.setattr(sys, "argv", ["mitm-load"])

    with pytest.raises(RuntimeError) as failure:
        main()
    message = str(failure.value)
    assert "requests failed at concurrency [200]" in message
    assert f"artifact={json.dumps(document, sort_keys=True)}" in message
    assert '"resources.total.cpu_seconds": 12.5' in message
    assert '"resources.total.peak_rss_bytes": 1048576' in message


def test_failed_dns_load_preserves_the_guest_proxy_log(monkeypatch) -> None:
    document = {
        "dns_load": {
            "concurrency_levels": [
                {"concurrency": 512, "total_requests": 600, "errors": 1, "rps": 50.0},
            ]
        }
    }
    exec_commands = []

    def post(route, body=None, **kwargs):
        if route == "/vms/create":
            return {"id": "bench-test-vm"}
        assert body is not None
        command = body["command"]
        exec_commands.append(command)
        if command == "cat /tmp/capsem-benchmark.json":
            output = json.dumps(document)
        elif command == (
            "cat /run/capsem-dns-proxy.log /proc/sys/net/core/rmem_max "
            "/proc/net/snmp /proc/net/udp /proc/net/softnet_stat"
        ):
            output = "dns query unanswered: queue saturated"
        else:
            output = "benchmark output"
        return {
            "exit_code": 0,
            "stdout": {"encoding": "utf8", "data": output},
        }

    client = SimpleNamespace(post=post, delete=lambda *args: None)
    main = collector()["main"]
    monkeypatch.setitem(
        main.__globals__,
        "ServiceInstance",
        lambda: SimpleNamespace(
            client=lambda: client,
            start=lambda: None,
            stop=lambda: None,
        ),
    )
    monkeypatch.setitem(
        main.__globals__, "wait_exec_ready", lambda *args, **kwargs: True
    )
    monkeypatch.setitem(
        main.__globals__, "process_tree_sampler", lambda service: ResourceSamplerStub()
    )
    monkeypatch.setattr(sys, "argv", ["dns-load"])

    with pytest.raises(RuntimeError) as failure:
        main()
    assert "guest_diagnostics='dns query unanswered: queue saturated'" in str(
        failure.value
    )
    assert exec_commands[-1] == (
        "cat /run/capsem-dns-proxy.log /proc/sys/net/core/rmem_max "
        "/proc/net/snmp /proc/net/udp /proc/net/softnet_stat"
    )


def test_guest_command_failure_preserves_resource_pressure(
    monkeypatch, tmp_path
) -> None:
    client = SimpleNamespace(
        post=lambda route, *args, **kwargs: (
            {"id": "bench-test-vm"}
            if route == "/vms/create"
            else {"exit_code": 1, "stderr": "load failed"}
        ),
        delete=lambda *args: None,
    )
    main = collector()["main"]
    monkeypatch.setitem(
        main.__globals__,
        "ServiceInstance",
        lambda: SimpleNamespace(
            client=lambda: client,
            start=lambda: None,
            stop=lambda: None,
            stop_and_read_log=lambda: "",
            tmp_dir=tmp_path,
        ),
    )
    import helpers.mock_server

    monkeypatch.setattr(
        helpers.mock_server,
        "start_mock_server",
        lambda: (
            object(),
            {"dns_udp_addr": "127.0.0.1:41000", "http_addr": "127.0.0.1:3713"},
        ),
    )
    monkeypatch.setattr(helpers.mock_server, "stop_process", lambda proc: None)
    monkeypatch.setitem(
        main.__globals__, "wait_exec_ready", lambda *args, **kwargs: True
    )

    class FailedRunSampler(ResourceSamplerStub):
        def stop(self) -> dict:
            return {
                "resources.total.cpu_seconds": {
                    "unit": "seconds",
                    "samples": [8.75],
                }
            }

    monkeypatch.setitem(
        main.__globals__, "process_tree_sampler", lambda service: FailedRunSampler()
    )
    monkeypatch.setattr(sys, "argv", ["mitm-load"])

    with pytest.raises(RuntimeError) as failure:
        main()
    message = str(failure.value)
    assert "capsem-bench mitm-load failed: exit=1" in message
    assert '"resources.total.cpu_seconds": 8.75' in message
