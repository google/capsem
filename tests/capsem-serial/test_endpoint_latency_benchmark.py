"""Host-side endpoint latency benchmark for the service and gateway.

The TUI depends on these read paths feeling instant while multiple VMs are
alive. This benchmark keeps the gate focused on endpoint latency, so it uses
raw persistent/fresh HTTP clients instead of the curl-based correctness helpers.
"""

import http.client
import json
import math
import os
import re
import socket
import time
import uuid
from pathlib import Path

import pytest

from helpers.benchmark_artifacts import (
    benchmark_arch,
    benchmark_output_path,
    enrich_benchmark_artifact,
)
from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.gateway import GatewayInstance
from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = [pytest.mark.serial, pytest.mark.benchmark]

PROJECT_ROOT = Path(__file__).parent.parent.parent
VM_COUNT = int(os.environ.get("CAPSEM_ENDPOINT_BENCH_VM_COUNT", "8"))
GLOBAL_ITERATIONS = int(os.environ.get("CAPSEM_ENDPOINT_BENCH_GLOBAL_RUNS", "16"))
VM_ITERATIONS = int(os.environ.get("CAPSEM_ENDPOINT_BENCH_VM_RUNS", "4"))
GATEWAY_ITERATIONS = int(os.environ.get("CAPSEM_ENDPOINT_BENCH_GATEWAY_RUNS", "32"))

GLOBAL_GATE_P95_MS = float(os.environ.get("CAPSEM_ENDPOINT_BENCH_GLOBAL_P95_MS", "3.0"))
GLOBAL_GATE_MAX_MS = float(os.environ.get("CAPSEM_ENDPOINT_BENCH_GLOBAL_MAX_MS", "10.0"))
VM_GATE_P95_MS = float(os.environ.get("CAPSEM_ENDPOINT_BENCH_VM_P95_MS", "12.0"))
VM_GATE_MAX_MS = float(os.environ.get("CAPSEM_ENDPOINT_BENCH_VM_MAX_MS", "35.0"))
GATEWAY_GATE_P95_MS = float(os.environ.get("CAPSEM_ENDPOINT_BENCH_GATEWAY_P95_MS", "2.0"))
GATEWAY_GATE_MAX_MS = float(os.environ.get("CAPSEM_ENDPOINT_BENCH_GATEWAY_MAX_MS", "8.0"))

GLOBAL_ENDPOINT_P95_OVERRIDES_MS = {
    "/settings": float(os.environ.get("CAPSEM_ENDPOINT_BENCH_SETTINGS_P95_MS", "7.0")),
    "/profiles": float(os.environ.get("CAPSEM_ENDPOINT_BENCH_PROFILES_P95_MS", "7.0")),
}
VM_ENDPOINT_P95_PREFIX_OVERRIDES_MS = {
    "/logs/": float(os.environ.get("CAPSEM_ENDPOINT_BENCH_LOGS_P95_MS", "30.0")),
}

GLOBAL_ENDPOINTS = (
    "/version",
    "/list",
    "/stats",
    "/settings",
    "/settings/presets",
    "/profiles",
    "/profiles/catalog",
    "/rules",
    "/enforcement",
    "/enforcement/stats",
    "/detection",
    "/detection/stats",
    "/confirm/pending",
    "/skills",
    "/setup/state",
    "/setup/assets",
    "/mcp/connectors",
)

VM_ENDPOINTS = (
    "/info/{id}",
    "/logs/{id}",
    "/history/{id}",
    "/history/{id}/counts",
    "/history/{id}/processes",
    "/history/{id}/transcript",
    "/files/{id}",
    "/sessions/{id}/policy-contexts",
)

GATEWAY_ENDPOINTS = (
    ("/health", False),
    ("/token", False),
    ("/status", True),
)


def _project_version():
    cargo = PROJECT_ROOT / "Cargo.toml"
    match = re.search(r'^version\s*=\s*"([^"]+)"', cargo.read_text(), re.MULTILINE)
    return match.group(1) if match else "unknown"


def _save_benchmark(data):
    version = _project_version()
    arch = benchmark_arch()
    out_path = benchmark_output_path(PROJECT_ROOT, "endpoint-latency", version, arch)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    data = enrich_benchmark_artifact(
        data,
        project_root=PROJECT_ROOT,
        project_version=version,
        arch=arch,
        command="uv run pytest tests/capsem-serial/test_endpoint_latency_benchmark.py -xvs",
    )
    out_path.write_text(json.dumps(data, indent=2))
    print(f"Endpoint latency benchmark saved to {out_path}")


def _uds_get(socket_path, path):
    started = time.perf_counter()
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
        sock.settimeout(10)
        sock.connect(str(socket_path))
        request = (
            f"GET {path} HTTP/1.1\r\n"
            "Host: localhost\r\n"
            "Connection: close\r\n"
            "\r\n"
        ).encode()
        sock.sendall(request)
        chunks = []
        while True:
            chunk = sock.recv(65536)
            if not chunk:
                break
            chunks.append(chunk)
    elapsed_ms = (time.perf_counter() - started) * 1000
    raw = b"".join(chunks)
    status_line = raw.split(b"\r\n", 1)[0].decode("ascii", errors="replace")
    parts = status_line.split()
    if len(parts) < 2 or not parts[1].isdigit():
        raise AssertionError(f"invalid HTTP response for {path}: {status_line!r}")
    status = int(parts[1])
    return status, elapsed_ms


def _gateway_get(connection, token, path, use_auth):
    headers = {"Connection": "keep-alive"}
    if use_auth:
        headers["Authorization"] = f"Bearer {token}"
    started = time.perf_counter()
    connection.request("GET", path, headers=headers)
    response = connection.getresponse()
    response.read()
    elapsed_ms = (time.perf_counter() - started) * 1000
    return response.status, elapsed_ms


def _percentile(values, percentile):
    ordered = sorted(values)
    if not ordered:
        return 0.0
    index = math.ceil((percentile / 100) * len(ordered)) - 1
    return ordered[max(0, min(index, len(ordered) - 1))]


def _summary(values):
    ordered = sorted(values)
    return {
        "count": len(ordered),
        "min_ms": round(ordered[0], 3),
        "p50_ms": round(_percentile(ordered, 50), 3),
        "p95_ms": round(_percentile(ordered, 95), 3),
        "p99_ms": round(_percentile(ordered, 99), 3),
        "max_ms": round(ordered[-1], 3),
    }


def _measure_service_group(socket_path, endpoints, iterations):
    results = {}
    for endpoint in endpoints:
        status, _ = _uds_get(socket_path, endpoint)
        assert 200 <= status < 300, f"{endpoint} warmup returned HTTP {status}"
        values = []
        for _ in range(iterations):
            status, elapsed_ms = _uds_get(socket_path, endpoint)
            assert 200 <= status < 300, f"{endpoint} returned HTTP {status}"
            values.append(elapsed_ms)
        results[endpoint] = _summary(values)
    return results


def _measure_gateway(gateway):
    results = {}
    conn = http.client.HTTPConnection("127.0.0.1", gateway.port, timeout=10)
    try:
        for endpoint, use_auth in GATEWAY_ENDPOINTS:
            status, _ = _gateway_get(conn, gateway.token, endpoint, use_auth)
            assert 200 <= status < 300, f"gateway {endpoint} warmup returned HTTP {status}"
            values = []
            for _ in range(GATEWAY_ITERATIONS):
                status, elapsed_ms = _gateway_get(conn, gateway.token, endpoint, use_auth)
                assert 200 <= status < 300, f"gateway {endpoint} returned HTTP {status}"
                values.append(elapsed_ms)
            results[endpoint] = _summary(values)
    finally:
        conn.close()
    return results


def _provision_vms(client, names):
    for name in names:
        client.post(
            "/provision",
            {
                "name": name,
                "persistent": False,
                "ram_mb": DEFAULT_RAM_MB,
                "cpus": DEFAULT_CPUS,
            },
            timeout=90,
        )
        assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT), f"{name} not exec-ready"


def _check_gates(results):
    failures = []
    gates = {
        "service_global": (GLOBAL_GATE_P95_MS, GLOBAL_GATE_MAX_MS),
        "service_vm": (VM_GATE_P95_MS, VM_GATE_MAX_MS),
        "gateway": (GATEWAY_GATE_P95_MS, GATEWAY_GATE_MAX_MS),
    }
    for group, endpoints in results["groups"].items():
        default_p95_gate, max_gate = gates[group]
        for endpoint, stats in endpoints.items():
            p95_gate = _p95_gate_for(group, endpoint, default_p95_gate)
            if stats["p95_ms"] > p95_gate or stats["max_ms"] > max_gate:
                failures.append(
                    f"{group} {endpoint}: p95={stats['p95_ms']}ms"
                    f" max={stats['max_ms']}ms gates p95<={p95_gate}ms max<={max_gate}ms"
                )
    assert not failures, "endpoint latency gate failed:\n" + "\n".join(failures)


def _p95_gate_for(group, endpoint, default_p95_gate):
    if group == "service_global":
        return GLOBAL_ENDPOINT_P95_OVERRIDES_MS.get(endpoint, default_p95_gate)
    if group == "service_vm":
        for prefix, p95_gate in VM_ENDPOINT_P95_PREFIX_OVERRIDES_MS.items():
            if endpoint.startswith(prefix):
                return p95_gate
    return default_p95_gate


def test_endpoint_latency_benchmark_8_live_vms():
    suffix = uuid.uuid4().hex[:8]
    vm_names = [f"epbench-{suffix}-{i}" for i in range(VM_COUNT)]
    svc = ServiceInstance()
    svc.start()
    client = svc.client()
    gateway = None
    try:
        _provision_vms(client, vm_names)

        vm_paths = [
            template.format(id=vm_name)
            for vm_name in vm_names
            for template in VM_ENDPOINTS
        ]
        service_global = _measure_service_group(
            svc.uds_path,
            GLOBAL_ENDPOINTS,
            GLOBAL_ITERATIONS,
        )
        service_vm = _measure_service_group(svc.uds_path, vm_paths, VM_ITERATIONS)

        gateway = GatewayInstance(svc.uds_path)
        gateway.start()
        gateway_results = _measure_gateway(gateway)

        result = {
            "version": _project_version(),
            "timestamp": time.time(),
            "vm_count": VM_COUNT,
            "iterations": {
                "service_global": GLOBAL_ITERATIONS,
                "service_vm": VM_ITERATIONS,
                "gateway": GATEWAY_ITERATIONS,
            },
            "gates": {
                "service_global": {
                    "p95_ms": GLOBAL_GATE_P95_MS,
                    "max_ms": GLOBAL_GATE_MAX_MS,
                },
                "service_vm": {
                    "p95_ms": VM_GATE_P95_MS,
                    "max_ms": VM_GATE_MAX_MS,
                },
                "gateway": {
                    "p95_ms": GATEWAY_GATE_P95_MS,
                    "max_ms": GATEWAY_GATE_MAX_MS,
                },
                "endpoint_p95_overrides_ms": {
                    "service_global": GLOBAL_ENDPOINT_P95_OVERRIDES_MS,
                    "service_vm_prefix": VM_ENDPOINT_P95_PREFIX_OVERRIDES_MS,
                },
            },
            "groups": {
                "service_global": service_global,
                "service_vm": service_vm,
                "gateway": gateway_results,
            },
        }

        for group, endpoints in result["groups"].items():
            slowest = max(endpoints.items(), key=lambda item: item[1]["p95_ms"])
            print(
                f"{group}: slowest p95 {slowest[0]} = "
                f"{slowest[1]['p95_ms']}ms max={slowest[1]['max_ms']}ms"
            )

        _save_benchmark(result)
        _check_gates(result)
    finally:
        if gateway is not None:
            gateway.stop()
        for name in vm_names:
            try:
                client.delete(f"/delete/{name}", timeout=30)
            except Exception:
                pass
        svc.stop()
