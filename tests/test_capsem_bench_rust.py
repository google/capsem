from __future__ import annotations

import json
import subprocess
from pathlib import Path

from helpers.mock_server import start_mock_server, stop_process


PROJECT_ROOT = Path(__file__).resolve().parents[1]
BENCH_BINARY = PROJECT_ROOT / "target" / "debug" / "capsem-bench"


def _ensure_bench_binary() -> None:
    subprocess.run(
        ["cargo", "build", "-p", "capsem-bench"],
        cwd=PROJECT_ROOT,
        check=True,
    )
    assert BENCH_BINARY.exists()


def _run_protocol(base_url: str, dns_udp_addr: str, out: Path, lane: str) -> dict:
    subprocess.run(
        [
            str(BENCH_BINARY),
            "protocol",
            "--base-url",
            base_url,
            "--dns-udp-addr",
            dns_udp_addr,
            "--requests",
            "200",
            "--concurrency",
            "16",
            "--scenarios",
            "tiny_http,model_json_response,credential_response,dns_local_nxdomain",
            "--lane",
            lane,
            "--json-out",
            str(out),
        ],
        cwd=PROJECT_ROOT,
        check=True,
        stdout=subprocess.DEVNULL,
    )
    return json.loads(out.read_text())


def test_rust_capsem_bench_protocol_and_delta_contract(tmp_path: Path) -> None:
    _ensure_bench_binary()
    proc = None
    try:
        proc, ready = start_mock_server()
        host = _run_protocol(
            ready["base_url"],
            ready["dns_udp_addr"],
            tmp_path / "host.json",
            "host_direct",
        )
        guest = _run_protocol(
            ready["base_url"],
            ready["dns_udp_addr"],
            tmp_path / "guest.json",
            "guest_capsem",
        )

        result = host["mock_server_protocol"]
        assert result["version"] == "1.1-rust"
        assert result["lane"] == "host_direct"
        assert result["dns_udp_addr"] == ready["dns_udp_addr"]
        assert result["total_requests"] == 200
        assert result["concurrency"] == 16
        assert result["selected_scenarios"] == [
            "tiny_http",
            "model_json_response",
            "credential_response",
            "dns_local_nxdomain",
        ]

        rows = {row["name"]: row for row in result["scenarios"]}
        assert rows["tiny_http"]["successful"] == 200
        assert rows["tiny_http"]["failed"] == 0
        assert rows["tiny_http"]["latency_ms"]["p95"] >= 0
        assert rows["model_json_response"]["successful"] == 200
        assert rows["model_json_response"]["failed"] == 0
        assert rows["credential_response"]["successful"] == 200
        assert rows["credential_response"]["secret_shaped_fixture_seen"] is True
        assert rows["credential_response"]["raw_secret_stored_in_result"] is False
        assert rows["dns_local_nxdomain"]["successful"] == 200
        assert rows["dns_local_nxdomain"]["failed"] == 0
        assert rows["dns_local_nxdomain"]["path"] == "load-test.capsem-bogus"
        assert rows["dns_local_nxdomain"]["body_kind"] == "dns_udp"
        assert rows["dns_local_nxdomain"]["errors"] == {}
        assert "capsem_test_api_key" not in json.dumps(host)

        delta_out = tmp_path / "delta.json"
        subprocess.run(
            [
                str(BENCH_BINARY),
                "delta",
                "--host",
                str(tmp_path / "host.json"),
                "--guest",
                str(tmp_path / "guest.json"),
                "--json-out",
                str(delta_out),
            ],
            cwd=PROJECT_ROOT,
            check=True,
            stdout=subprocess.DEVNULL,
        )
        delta = json.loads(delta_out.read_text())
        assert delta["benchmark"] == "capsem-bench-rs-delta"
        assert delta["abstraction_delta"]["host_lane"] == "host_direct"
        assert delta["abstraction_delta"]["guest_lane"] == "guest_capsem"
        delta_rows = {
            row["name"]: row for row in delta["abstraction_delta"]["scenarios"]
        }
        assert set(delta_rows) == {
            "tiny_http",
            "model_json_response",
            "credential_response",
            "dns_local_nxdomain",
        }
        for row in delta_rows.values():
            assert row["rps_ratio_guest_over_host"] > 0
            assert row["throughput_ratio_guest_over_host"] >= 0
            assert "p95_delta_ms" in row
            assert row["error_delta"] == 0
    finally:
        stop_process(proc)
