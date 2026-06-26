"""CLI entry point for legacy non-HTTP capsem-bench modes."""

import json
import os
import subprocess
import sys
import time

from .helpers import console

VALID_MODES = (
    "disk",
    "rootfs",
    "storage",
    "startup",
    "snapshot",
    "mitm-load",
    "mcp-load",
    "dns-load",
    "all",
)

RUST_ONLY_MODES = ("http", "throughput", "protocol")
MOCK_SERVER_PROTOCOL_BASE_URL_ENV = "CAPSEM_MOCK_SERVER_BASE_URL"
MOCK_SERVER_DNS_UDP_ADDR_ENV = "CAPSEM_MOCK_SERVER_DNS_UDP_ADDR"
RUST_BENCH = "/usr/local/bin/capsem-bench-rs"


def main():
    args = sys.argv[1:]
    mode = args[0] if args else "all"

    if mode == "protocol":
        sys.exit(_run_rust_protocol(args))

    if mode in ("http", "throughput"):
        console.print(
            f"ERROR: capsem-bench {mode} is retired from Python; "
            f"use capsem-bench-rs {mode}"
        )
        sys.exit(127)

    if mode in ("-h", "--help"):
        console.print(
            "Usage: capsem-bench "
            "[disk|rootfs|storage|startup|snapshot|mitm-load|mcp-load|dns-load|all] "
            "[OPTIONS]"
        )
        console.print()
        console.print("Commands:")
        console.print("  disk                Scratch disk I/O benchmarks")
        console.print("  rootfs              Rootfs read I/O benchmarks")
        console.print("  storage             Rootfs/workspace/tmpfs/overlay storage split")
        console.print("  startup             CLI cold-start latency")
        console.print("  snapshot            Snapshot ops (create/list/revert/delete via MCP)")
        console.print("  mitm-load [C[,C]] [SECONDS]  MITM proxy load test")
        console.print("  mcp-load [C[,C]] [SECONDS]   MCP path load test")
        console.print("  dns-load [C[,C]] [SECONDS]   DNS proxy load test")
        console.print("  all                 Run all legacy non-HTTP benchmarks (default)")
        console.print()
        console.print("Rust-only benchmarks:")
        console.print("  protocol            Delegates to capsem-bench-rs protocol")
        console.print("  capsem-bench-rs protocol       HTTP/model/MCP/DNS protocol benchmark")
        console.print()
        console.print("Environment:")
        console.print("  CAPSEM_BENCH_DIR      Test directory (default: /root)")
        console.print("  CAPSEM_BENCH_SIZE_MB  Write test size in MB (default: 256)")
        console.print("  CAPSEM_MOCK_SERVER_BASE_URL       Base URL for Rust protocol scenarios")
        console.print("  CAPSEM_MOCK_SERVER_DNS_UDP_ADDR   UDP address for Rust DNS scenarios")
        console.print("  CAPSEM_BENCH_CONCURRENCY          Load concurrency, e.g. 64 or 1,64")
        console.print("  CAPSEM_BENCH_DURATION_S           Seconds per load level")
        console.print("  CAPSEM_BENCH_TOTAL_REQUESTS       Total requests per Rust protocol scenario")
        console.print("  CAPSEM_BENCH_SCENARIOS            Comma-separated Rust protocol scenarios")
        console.print("  CAPSEM_STORAGE_BENCH_PATHS      Storage paths for split diagnostics")
        console.print("  CAPSEM_STORAGE_BENCH_SIZE_MB    Storage split write size in MB")
        console.print("  CAPSEM_STORAGE_IO_PROFILE_SIZE_MB    Storage IOPS profile size")
        console.print("  CAPSEM_STORAGE_IO_PROFILE_RANDOM_OPS Storage random I/O operations")
        sys.exit(0)

    if mode not in VALID_MODES:
        console.print(f"Unknown command: {mode}")
        console.print("Run 'capsem-bench --help' for usage.")
        sys.exit(1)

    output = {
        "version": "0.3.0",
        "timestamp": time.time(),
        "hostname": os.uname().nodename,
    }

    if mode in ("disk", "all"):
        from .disk import disk_bench
        output["disk"] = disk_bench()

    if mode in ("rootfs", "all"):
        from .rootfs import rootfs_bench
        output["rootfs"] = rootfs_bench()

    if mode in ("storage", "all"):
        from .storage import storage_bench
        output["storage"] = storage_bench()

    if mode in ("startup", "all"):
        from .startup import startup_bench
        output["startup"] = startup_bench()

    if mode in ("snapshot", "all"):
        from .snapshot import snapshot_bench
        output["snapshot"] = snapshot_bench()

    if mode == "all" and os.environ.get(MOCK_SERVER_PROTOCOL_BASE_URL_ENV):
        protocol_output = _run_rust_protocol_artifact()
        http_output = _run_rust_protocol_artifact(scenarios="tiny_http")
        throughput_output = _run_rust_protocol_artifact(
            scenarios="http_10mb",
            requests="1",
            concurrency="1",
        )
        output.update(
            _legacy_network_sections_from_rust(
                protocol_output,
                http_output=http_output,
                throughput_output=throughput_output,
            )
        )

    # mitm-load runs only when explicitly requested -- it's a long-running
    # proxy stress test and would dominate `capsem-bench all`.
    if mode == "mitm-load":
        from .mitm_load import mitm_load_bench
        from .load_harness import parse_concurrency_levels
        c = parse_concurrency_levels(args[1]) if len(args) > 1 else None
        duration = float(args[2]) if len(args) > 2 else None
        output["mitm_load"] = mitm_load_bench(
            concurrency_levels=c, duration_s=duration
        )

    if mode == "mcp-load":
        from .mcp_load import mcp_load_bench
        from .load_harness import parse_concurrency_levels
        c = parse_concurrency_levels(args[1]) if len(args) > 1 else None
        duration = float(args[2]) if len(args) > 2 else None
        output["mcp_load"] = mcp_load_bench(
            concurrency_levels=c, duration_s=duration
        )

    if mode == "dns-load":
        from .dns_load import dns_load_bench
        from .load_harness import parse_concurrency_levels
        c = parse_concurrency_levels(args[1]) if len(args) > 1 else None
        duration = float(args[2]) if len(args) > 2 else None
        output["dns_load"] = dns_load_bench(
            concurrency_levels=c, duration_s=duration
        )

    json_path = "/tmp/capsem-benchmark.json"
    with open(json_path, "w") as f:
        json.dump(output, f, indent=2)
    console.print(f"\nJSON results saved to {json_path}")


def _run_rust_protocol(args):
    if not os.path.exists(RUST_BENCH):
        console.print(f"ERROR: {RUST_BENCH} is required for capsem-bench protocol")
        return 127
    completed = subprocess.run([RUST_BENCH, *args], check=False)
    return completed.returncode


def _run_rust_protocol_artifact(scenarios=None, requests=None, concurrency=None):
    if not os.path.exists(RUST_BENCH):
        raise RuntimeError(f"{RUST_BENCH} is required for capsem-bench all protocol section")
    scenarios = scenarios or os.environ.get("CAPSEM_BENCH_SCENARIOS")
    if not scenarios:
        scenarios = "model_json_response,credential_response,mcp_tools_list,mcp_tool_call,dns_local_nxdomain"
    command = [
        RUST_BENCH,
        "protocol",
        "--lane",
        "guest_capsem",
        "--scenarios",
        scenarios,
    ]
    if os.environ.get(MOCK_SERVER_PROTOCOL_BASE_URL_ENV):
        command.extend(["--base-url", os.environ[MOCK_SERVER_PROTOCOL_BASE_URL_ENV]])
    if os.environ.get(MOCK_SERVER_DNS_UDP_ADDR_ENV):
        command.extend(["--dns-udp-addr", os.environ[MOCK_SERVER_DNS_UDP_ADDR_ENV]])
    if requests is not None:
        command.extend(["--requests", str(requests)])
    elif os.environ.get("CAPSEM_BENCH_TOTAL_REQUESTS"):
        command.extend(["--requests", os.environ["CAPSEM_BENCH_TOTAL_REQUESTS"]])
    if concurrency is not None:
        command.extend(["--concurrency", str(concurrency)])
    elif os.environ.get("CAPSEM_BENCH_CONCURRENCY"):
        command.extend(["--concurrency", os.environ["CAPSEM_BENCH_CONCURRENCY"]])
    if os.environ.get("CAPSEM_BENCH_TIMEOUT_MS"):
        command.extend(["--timeout-ms", os.environ["CAPSEM_BENCH_TIMEOUT_MS"]])
    completed = subprocess.run(command, check=False, capture_output=True, text=True)
    if completed.returncode != 0:
        raise RuntimeError(
            f"capsem-bench-rs protocol failed with {completed.returncode}: "
            f"{completed.stderr or completed.stdout}"
        )
    with open("/tmp/capsem-benchmark.json") as f:
        return json.load(f)


def _legacy_network_sections_from_rust(
    protocol_output,
    http_output=None,
    throughput_output=None,
):
    protocol = protocol_output["mock_server_protocol"]
    http_protocol = (http_output or protocol_output)["mock_server_protocol"]
    throughput_protocol = (throughput_output or protocol_output)["mock_server_protocol"]
    http_scenarios = {row["name"]: row for row in http_protocol["scenarios"]}
    throughput_scenarios = {
        row["name"]: row for row in throughput_protocol["scenarios"]
    }
    result = {"mock_server_protocol": protocol}
    if "tiny_http" in http_scenarios:
        row = http_scenarios["tiny_http"]
        result["http"] = {
            "url": f"{http_protocol['base_url']}{row['path']}",
            "total_requests": row["total_requests"],
            "successful": row["successful"],
            "failed": row["failed"],
            "requests_per_sec": row["requests_per_sec"],
            "latency_ms": row["latency_ms"],
            "bytes_per_sec": row["bytes_per_sec"],
            "transfer_bytes": row["transfer_bytes"],
            "source": "local",
        }
    if "http_10mb" in throughput_scenarios:
        row = throughput_scenarios["http_10mb"]
        result["throughput"] = {
            "url": f"{throughput_protocol['base_url']}{row['path']}",
            "http_code": 200 if row["failed"] == 0 else 500,
            "size_bytes": row["transfer_bytes"],
            "throughput_mbps": round(row["bytes_per_sec"] / (1024 * 1024), 1),
            "source": "local",
            "duration_s": row["total_duration_ms"] / 1000.0,
        }
    return result


if __name__ == "__main__":
    main()
