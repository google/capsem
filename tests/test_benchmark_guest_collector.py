"""The shared guest collector preserves hermetic controls and diagnostics."""

from __future__ import annotations

import json
import os
import runpy
import shlex
import sys
from pathlib import Path
from types import SimpleNamespace

from capsem_builder.gate import config as gate_config

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)
COLLECTOR = CONFIG.path(CONFIG.benchmark.run.collectors) / "_guest"


def collector() -> dict:
    return runpy.run_path(str(COLLECTOR))


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


def test_a_level_where_every_request_failed_is_not_a_measurement() -> None:
    failed = collector()["failed_load_levels"]
    section = {
        "concurrency_levels": [
            {"concurrency": 1, "total_requests": 700, "errors": 700},
            {"concurrency": 10, "total_requests": 6000, "errors": 12},
            {"concurrency": 50, "total_requests": 0, "errors": 0},
            {"concurrency": 200, "total_requests": 100, "errors": 250},
        ]
    }
    assert failed(section) == [1, 200]
    assert failed({"concurrency_levels": []}) == []
    assert failed({}) == []


def test_load_dimensions_get_more_guest_cpus_bounded_by_the_host() -> None:
    guest_cpus = collector()["guest_cpus"]
    assert guest_cpus("disk", host_cpus=16) == 2, "non-load modes keep the default"
    assert guest_cpus("mitm-load", host_cpus=16) == 8
    assert guest_cpus("dns-load", host_cpus=64) == 8, "capped"
    assert guest_cpus("mcp-load", host_cpus=6) == 3
    assert guest_cpus("mitm-load", host_cpus=2) == 2, "never below the default"


def test_guest_result_preserves_error_counts_and_concurrency(monkeypatch, capsys) -> None:
    document = {"dns_load": {"concurrency_levels": [
        {"concurrency": 200, "total_requests": 500, "errors": 3, "rps": 50.0},
    ]}}
    stopped = []
    client = SimpleNamespace(
        post=lambda *args, **kwargs: {"exit_code": 0, "stdout": json.dumps(document)},
        delete=lambda *args: None,
    )
    main = collector()["main"]
    monkeypatch.setitem(main.__globals__, "ServiceInstance", lambda: SimpleNamespace(
        client=lambda: client, start=lambda: None, stop=lambda: stopped.append(True),
    ))
    monkeypatch.setitem(main.__globals__, "wait_exec_ready", lambda *args, **kwargs: True)
    monkeypatch.setattr(sys, "argv", ["dns-load"])

    assert main() == 0
    result = json.loads(capsys.readouterr().out)
    assert json.loads(result["sidecar"]) == document
    assert result["metrics"] == {
        "concurrency_levels.0.rps": {"unit": "requests_per_second", "samples": [50.0]},
    }
    assert stopped == [True]
