"""CLI entry point for capsem-bench."""

import json
import os
import sys
import time

from .helpers import console

VALID_MODES = ("disk", "rootfs", "startup", "http", "throughput", "snapshot", "all")


def main():
    args = sys.argv[1:]
    mode = args[0] if args else "all"

    if mode in ("-h", "--help"):
        console.print("Usage: capsem-bench [disk|rootfs|startup|http|throughput|snapshot|all] [OPTIONS]")
        console.print()
        console.print("Commands:")
        console.print("  disk                Scratch disk I/O benchmarks")
        console.print("  rootfs              Rootfs read I/O benchmarks")
        console.print("  startup             CLI cold-start latency")
        console.print("  http [URL] [N] [C]  HTTP benchmarks (ab-style)")
        console.print("  throughput          100 MB download through MITM proxy")
        console.print("  snapshot            Snapshot ops (create/list/revert/delete via MCP)")
        console.print("  all                 Run all benchmarks (default)")
        console.print()
        console.print("Environment:")
        console.print("  CAPSEM_BENCH_DIR      Test directory (default: /root)")
        console.print("  CAPSEM_BENCH_SIZE_MB  Write test size in MB (default: 256)")
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

    # JSON to file (machine-readable)
    json_path = "/tmp/capsem-benchmark.json"
    with open(json_path, "w") as f:
        json.dump(output, f, indent=2)
    console.print(f"\nJSON results saved to {json_path}")


if __name__ == "__main__":
    main()
