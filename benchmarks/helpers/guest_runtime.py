"""Runtime controls shared by Capsem's in-guest benchmark collectors."""

from __future__ import annotations

import json
import os
import shlex

import psutil
from helpers.constants import DEFAULT_CPUS
from helpers.process_resources import ProcessTreeSampler
from helpers.service import ServiceInstance, exec_output_text

FORWARDED_ENV = (
    "CAPSEM_BENCH_CONCURRENCY",
    "CAPSEM_BENCH_DURATION_S",
    "CAPSEM_BENCH_MITM_TARGET",
    "CAPSEM_BENCH_SCENARIOS",
    "CAPSEM_BENCH_TIMEOUT_MS",
    "CAPSEM_BENCH_TOTAL_REQUESTS",
    "CAPSEM_MOCK_SERVER_BASE_URL",
    "CAPSEM_MOCK_SERVER_DNS_UDP_ADDR",
    "CAPSEM_STORAGE_BENCH_PATHS",
    "CAPSEM_STORAGE_BENCH_SIZE_MB",
    "CAPSEM_STORAGE_IO_PROFILE_SIZE_MB",
    "CAPSEM_STORAGE_IO_PROFILE_RANDOM_OPS",
)
LOAD_MODES = ("dns-load", "mitm-load", "mcp-load", "protocol")
PROXY_ERROR_MODES = ("mitm-load", "protocol")
LOAD_GUEST_CPUS_MAX = 8
PROTOCOL_COLLECTOR_PATH = "/tmp/capsem-benchmark-collector.json"

MITM_FIXTURE_HOST = "fixture.capsem.test"
MITM_FIXTURE_TARGET = f"https://{MITM_FIXTURE_HOST}/tiny"
DNS_FAILURE_DIAGNOSTIC_COMMAND = (
    "cat /run/capsem-dns-proxy.log /proc/sys/net/core/rmem_max "
    "/proc/net/snmp /proc/net/udp /proc/net/softnet_stat"
)


def guest_cpus(mode: str, host_cpus: int | None = None) -> int:
    """Give load generators enough guest CPU without consuming the host."""
    if mode not in LOAD_MODES:
        return DEFAULT_CPUS
    host = host_cpus if host_cpus is not None else (os.cpu_count() or DEFAULT_CPUS)
    return max(DEFAULT_CPUS, min(LOAD_GUEST_CPUS_MAX, host // 2))


def process_tree_sampler(service: ServiceInstance) -> ProcessTreeSampler:
    """Measure the service and every confined per-VM descendant."""
    return ProcessTreeSampler(psutil.Process(service.proc.pid))


def mitm_fixture_corp_toml(ready: dict) -> str:
    """Route guest DNS and the fixture host to the mock server."""
    return (
        "[network.dns]\n"
        f"upstreams = [{json.dumps(ready['dns_udp_addr'])}]\n"
        "\n"
        f'[network.upstream_overrides."{MITM_FIXTURE_HOST}:443"]\n'
        f"dial = {json.dumps(ready['http_addr'])}\n"
        'protocol = "http"\n'
    )


def guest_command(mode: str, *, io_limits=None) -> str:
    """Build one safely quoted command with owned benchmark controls."""
    assignments = [
        f"{name}={value}"
        for name in FORWARDED_ENV
        if (value := os.environ.get(name)) is not None
    ]
    if mode in {"disk", "storage"} and io_limits is not None:
        assignments.extend((
            f"CAPSEM_BENCH_MAX_SAMPLES_PER_METRIC={io_limits.max_samples_per_metric}",
            f"CAPSEM_BENCH_MAX_SAMPLES_PER_RUN={io_limits.max_samples_per_run}",
            f"CAPSEM_BENCH_MAX_COLLECTOR_BYTES={io_limits.max_collector_bytes}",
        ))
    command = ["env", *assignments, "capsem-bench", mode]
    if mode == "protocol":
        command.extend(
            ["--lane", "guest_capsem", "--collector-out", PROTOCOL_COLLECTOR_PATH]
        )
    return shlex.join(command)


def failed_load_diagnostics(service, client, vm_id: str, mode: str) -> str:
    """Capture guest state that teardown would otherwise discard."""
    if mode != "dns-load":
        return ""
    result = client.post(
        f"/vms/{vm_id}/exec",
        {
            "target": "vm",
            "command": DNS_FAILURE_DIAGNOSTIC_COMMAND,
            "timeout_secs": 15,
        },
        timeout=25,
    )
    if not result or result.get("exit_code") != 0:
        diagnostics = f"diagnostic command failed: {json.dumps(result)[:2000]}"
    else:
        diagnostics = exec_output_text(result)
    diagnostic_path = getattr(service, "tmp_dir", None)
    if diagnostic_path is not None:
        (diagnostic_path / "dns-load-failure.txt").write_text(
            diagnostics, encoding="utf-8"
        )
    return diagnostics
