"""CLI entry point for capsem-bench."""

import json
import os
import sys
import time

from .helpers import console

VALID_MODES = (
    "disk", "rootfs", "storage", "startup", "http", "throughput", "snapshot",
    "protocol", "mitm-load", "mcp-load", "dns-load", "all",
)

MOCK_SERVER_PROTOCOL_BASE_URL_ENV = "CAPSEM_MOCK_SERVER_BASE_URL"


def _should_run_mock_server_protocol(mode):
    if mode == "protocol":
        return True
    return mode == "all" and bool(os.environ.get(MOCK_SERVER_PROTOCOL_BASE_URL_ENV))


def main():
    args = sys.argv[1:]
    mode = args[0] if args else "all"

    if mode in ("-h", "--help"):
        console.print(
            "Usage: capsem-bench "
            "[disk|rootfs|storage|startup|http|throughput|snapshot|protocol|all] "
            "[OPTIONS]"
        )
        console.print()
        console.print("Commands:")
        console.print("  disk                Scratch disk I/O benchmarks")
        console.print("  rootfs              Rootfs read I/O benchmarks")
        console.print("  storage             Rootfs/workspace/tmpfs/overlay storage split")
        console.print("  startup             CLI cold-start latency")
        console.print("  http [URL] [N] [C]  HTTP benchmarks (ab-style)")
        console.print("  throughput          100 MB download through MITM proxy")
        console.print("  snapshot            Snapshot ops (create/list/revert/delete via MCP)")
        console.print("  protocol            Local mock-server protocol benchmark")
        console.print("  mitm-load [C[,C]] [SECONDS]  MITM proxy load test")
        console.print("  mcp-load [C[,C]] [SECONDS]   MCP path load test")
        console.print("  dns-load [C[,C]] [SECONDS]   DNS proxy load test")
        console.print("  all                 Run all benchmarks (default)")
        console.print()
        console.print("Environment:")
        console.print("  CAPSEM_BENCH_DIR      Test directory (default: /root)")
        console.print("  CAPSEM_BENCH_SIZE_MB  Write test size in MB (default: 256)")
        console.print("  CAPSEM_MOCK_SERVER_BASE_URL  Base URL for protocol scenarios")
        console.print("  CAPSEM_BENCH_CONCURRENCY          Load concurrency, e.g. 64 or 1,64")
        console.print("  CAPSEM_BENCH_DURATION_S           Seconds per load level")
        console.print("  CAPSEM_BENCH_TOTAL_REQUESTS       Total requests per count scenario")
        console.print("  CAPSEM_BENCH_SCENARIOS            Comma-separated local mock-server protocol scenarios")
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

    if mode in ("http", "all"):
        from .http_bench import http_bench
        url = args[1] if len(args) > 1 and mode == "http" else None
        n = int(args[2]) if len(args) > 2 and mode == "http" else None
        c = int(args[3]) if len(args) > 3 and mode == "http" else None
        output["http"] = http_bench(url=url, total_requests=n, concurrency=c)

    if mode in ("throughput", "all"):
        from .throughput import throughput_bench
        output["throughput"] = throughput_bench()

    if mode in ("snapshot", "all"):
        from .snapshot import snapshot_bench
        output["snapshot"] = snapshot_bench()

    # Local protocol scenarios are part of the standard `all` benchmark when
    # the shared doctor/mock server is configured, and are also available as a
    # first-class `protocol` benchmark for release-scale network numbers.
    if _should_run_mock_server_protocol(mode):
        from .mock_server_protocol import mock_server_protocol_bench
        output["mock_server_protocol"] = mock_server_protocol_bench()

    # mitm-load runs only when explicitly requested -- it's a long-running
    # proxy stress test (default 10s per concurrency level x 4 levels = ~40s
    # of pure proxy load) and would dominate `capsem-bench all`.
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

    # dns-load runs only when explicitly requested -- same rationale
    # as mitm-load: ~40s of pure proxy stress per invocation, would
    # dominate `capsem-bench all`.
    if mode == "dns-load":
        from .dns_load import dns_load_bench
        from .load_harness import parse_concurrency_levels
        c = parse_concurrency_levels(args[1]) if len(args) > 1 else None
        duration = float(args[2]) if len(args) > 2 else None
        output["dns_load"] = dns_load_bench(
            concurrency_levels=c, duration_s=duration
        )

    # JSON to file (machine-readable)
    json_path = "/tmp/capsem-benchmark.json"
    with open(json_path, "w") as f:
        json.dump(output, f, indent=2)
    console.print(f"\nJSON results saved to {json_path}")


if __name__ == "__main__":
    main()
