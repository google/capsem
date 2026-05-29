"""CLI entry point for capsem-bench."""

import json
import os
import sys
import time

from .helpers import console

VALID_MODES = ("disk", "rootfs", "storage", "startup", "http", "throughput", "snapshot", "mitm-load", "mcp-load", "dns-load", "all")


def main():
    args = sys.argv[1:]
    mode = args[0] if args else "all"

    if mode in ("-h", "--help"):
        console.print("Usage: capsem-bench [disk|rootfs|storage|startup|http|throughput|snapshot|all] [OPTIONS]")
        console.print()
        console.print("Commands:")
        console.print("  disk                Scratch disk I/O benchmarks")
        console.print("  rootfs              Rootfs read I/O benchmarks")
        console.print("  storage             Rootfs/workspace/tmpfs/overlay storage split")
        console.print("  startup             CLI cold-start latency")
        console.print("  http [URL] [N] [C]  HTTP benchmarks (ab-style)")
        console.print("  throughput          100 MB download through MITM proxy")
        console.print("  snapshot            Snapshot ops (create/list/revert/delete via MCP)")
        console.print("  mitm-load           MITM proxy load test at 1/10/50/200 concurrency")
        console.print("  mcp-load            MCP path load test (echo tool) at 1/10/50/200 concurrency")
        console.print("  dns-load            DNS proxy load test at 1/10/50/200 concurrency")
        console.print("  all                 Run standard benchmarks, including storage split diagnostics")
        console.print()
        console.print("Environment:")
        console.print("  CAPSEM_BENCH_DIR                Test directory (default: /root)")
        console.print("  CAPSEM_BENCH_SIZE_MB            Write test size in MB (default: 256)")
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

    # mitm-load runs only when explicitly requested -- it's a long-running
    # proxy stress test (default 10s per concurrency level x 4 levels = ~40s
    # of pure proxy load) and would dominate `capsem-bench all`.
    if mode == "mitm-load":
        from .mitm_load import mitm_load_bench
        output["mitm_load"] = mitm_load_bench()

    if mode == "mcp-load":
        from .mcp_load import mcp_load_bench
        output["mcp_load"] = mcp_load_bench()

    # dns-load runs only when explicitly requested -- same rationale
    # as mitm-load: ~40s of pure proxy stress per invocation, would
    # dominate `capsem-bench all`.
    if mode == "dns-load":
        from .dns_load import dns_load_bench
        output["dns_load"] = dns_load_bench()

    # JSON to file (machine-readable)
    json_path = "/tmp/capsem-benchmark.json"
    with open(json_path, "w") as f:
        json.dump(output, f, indent=2)
    console.print(f"\nJSON results saved to {json_path}")


if __name__ == "__main__":
    main()
