"""Record in-VM capsem-bench output as a time-series baseline.

Provisions a fresh VM, runs `capsem-bench all`, pulls /tmp/capsem-benchmark.json
out via /exec, and archives it to benchmarks/capsem-bench/data_<version>_<arch>.json.

No gates yet -- we lack a stable baseline. Once 5-10 clean runs are on
disk per arch, per-category tolerances can be picked and promoted to
pytest asserts (mirroring OP_GATE_MS / FORK_GATE_MS in
test_lifecycle_benchmark.py).
"""

import json
import os
import re
import shlex
import time
import uuid
from pathlib import Path

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.benchmark_gates import validate_capsem_bench_result
from helpers.mock_server import start_mock_server, stop_process
from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = pytest.mark.serial

PROJECT_ROOT = Path(__file__).parent.parent.parent
RELEASE_PROTOCOL_SCENARIOS = ("model_json_response", "credential_response")
RELEASE_PROTOCOL_REQUESTS = 1_000
RELEASE_PROTOCOL_CONCURRENCY = 32


def _project_version():
    cargo = PROJECT_ROOT / "Cargo.toml"
    m = re.search(r'^version\s*=\s*"([^"]+)"', cargo.read_text(), re.MULTILINE)
    return m.group(1) if m else "unknown"


def _save(data):
    version = _project_version()
    arch = "arm64" if os.uname().machine == "arm64" else "x86_64"
    out_dir = PROJECT_ROOT / "benchmarks" / "capsem-bench"
    out_dir.mkdir(parents=True, exist_ok=True)
    out_path = out_dir / f"data_{version}_{arch}.json"
    with open(out_path, "w") as f:
        json.dump(data, f, indent=2)
    print(f"capsem-bench baseline archived to {out_path}")


def _assert_release_network_benchmarks_ran(data):
    http = data.get("http")
    assert isinstance(http, dict), "capsem-bench JSON missing http section"
    assert not http.get("skipped"), f"http benchmark skipped: {http}"
    assert http.get("successful") == http.get("total_requests"), http
    assert http.get("failed") == 0, http
    assert http.get("requests_per_sec", 0) > 0, http

    throughput = data.get("throughput")
    assert isinstance(throughput, dict), "capsem-bench JSON missing throughput section"
    assert not throughput.get("skipped"), f"throughput benchmark skipped: {throughput}"
    assert "error" not in throughput, throughput
    assert throughput.get("source") == "local", throughput
    assert throughput.get("size_bytes", 0) >= 10 * 1024 * 1024, throughput
    assert throughput.get("throughput_mbps", 0) > 0, throughput

    mock_server_protocol = data.get("mock_server_protocol")
    assert isinstance(mock_server_protocol, dict), "capsem-bench JSON missing mock_server_protocol section"
    assert not mock_server_protocol.get("skipped"), f"protocol benchmark skipped: {mock_server_protocol}"
    assert mock_server_protocol.get("total_requests", 0) > 0, mock_server_protocol
    for row in mock_server_protocol.get("scenarios", []):
        assert row["successful"] == row["total_requests"], row
        assert row["failed"] == 0, row


def test_capsem_bench_baseline():
    """Run capsem-bench all in a fresh VM, archive the JSON output."""
    upstream_proc = None
    upstream_proc, ready = start_mock_server()
    base_url = ready["base_url"]
    https_base_url = ready["https_base_url"]

    svc = ServiceInstance()
    svc.start()
    client = svc.client()
    name = f"bench-{uuid.uuid4().hex[:8]}"

    try:
        client.post("/vms/create", {
            "name": name,
            "ram_mb": DEFAULT_RAM_MB,
            "cpus": DEFAULT_CPUS,
        })
        assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT), (
            f"{name} not ready"
        )

        # Full suite: disk, rootfs, startup, http, throughput, snapshot.
        # 10-minute cap covers the 256MB disk tests + 10MB download +
        # 50 HTTP requests + snapshot ops without false-timing.
        command = shlex.join(
            [
                "env",
                f"CAPSEM_MOCK_SERVER_BASE_URL={base_url}",
                f"CAPSEM_MOCK_SERVER_HTTPS_BASE_URL={https_base_url}",
                f"CAPSEM_BENCH_TOTAL_REQUESTS={RELEASE_PROTOCOL_REQUESTS}",
                f"CAPSEM_BENCH_CONCURRENCY={RELEASE_PROTOCOL_CONCURRENCY}",
                f"CAPSEM_BENCH_SCENARIOS={','.join(RELEASE_PROTOCOL_SCENARIOS)}",
                "capsem-bench",
                "all",
            ]
        )
        resp = client.post(
            f"/vms/{name}/exec",
            {"command": command, "timeout_secs": 600},
            timeout=610,
        )
        assert resp and resp.get("exit_code") == 0, (
            f"capsem-bench all failed: exit={resp.get('exit_code') if resp else None}\n"
            f"stdout: {(resp or {}).get('stdout', '')[:500]}\n"
            f"stderr: {(resp or {}).get('stderr', '')[:500]}"
        )

        # capsem-bench writes /tmp/capsem-benchmark.json on success (see
        # guest/artifacts/capsem_bench/__main__.py). Pull it out before
        # the VM is torn down.
        resp = client.post(
            f"/vms/{name}/exec",
            {"command": "cat /tmp/capsem-benchmark.json", "timeout_secs": 15},
            timeout=20,
        )
        assert resp and resp.get("exit_code") == 0, (
            "capsem-bench did not produce /tmp/capsem-benchmark.json"
        )
        raw = resp.get("stdout", "").strip()
        data = json.loads(raw)
        validate_capsem_bench_result(data)
        _assert_release_network_benchmarks_ran(data)
        # Stamp host-side metadata so a future comparison helper can group
        # by arch and time without re-reading Cargo.toml.
        data["host_recorded_at"] = time.time()
        data["arch"] = os.uname().machine
        data["mock_server_base_url"] = base_url
        _save(data)
    finally:
        try:
            client.delete(f"/vms/{name}/delete")
        except Exception:
            pass
        svc.stop()
        stop_process(upstream_proc)
