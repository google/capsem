---
title: Benchmarks
description: Performance benchmarks for disk I/O, rootfs reads, CLI startup, network throughput, and snapshot operations.
---

Capsem includes `capsem-bench`, a Python benchmarking tool that runs inside the VM. It outputs rich tables to stderr and structured JSON to stdout.

## Running benchmarks

```bash
just bench                          # All benchmarks in VM
just run "capsem-bench snapshot"    # Snapshot operations only
just run "capsem-bench disk"        # Disk I/O only
just full-test                      # Full validation including benchmarks
```

## Benchmark categories

### Disk I/O (`disk`)

Sequential and random I/O on the scratch disk.

- **Sequential write** (1MB blocks): throughput in MB/s
- **Sequential read** (1MB blocks): throughput in MB/s
- **Random 4K write**: IOPS + throughput
- **Random 4K read**: IOPS + throughput

Configurable via `CAPSEM_BENCH_SIZE_MB` (default: 256).

### Rootfs reads (`rootfs`)

Read-only rootfs performance where binaries and libraries live.

- **Sequential read** of largest file in `/usr/bin`, `/usr/lib`, `/opt/ai-clis`
- **Random 4K reads** across multiple rootfs files (5000 samples)

### CLI startup (`startup`)

Cold-start latency for AI CLIs (3 runs each, page cache dropped between runs).

| CLI | Command |
|-----|---------|
| python3 | `python3 --version` |
| node | `node --version` |
| claude | `claude --version` |
| gemini | `gemini --version` |
| codex | `codex --version` |

### HTTP (`http`)

HTTP throughput through the MITM proxy. Default: 50 requests to `https://www.google.com/` with concurrency 5.

Reports: successful/failed requests, requests/sec, latency percentiles (p50, p95, p99).

Custom: `capsem-bench http URL N C` (URL, request count, concurrency).

### Proxy throughput (`throughput`)

Downloads 100MB through the MITM proxy and reports end-to-end throughput in MB/s.

### Snapshot operations (`snapshot`)

End-to-end snapshot latency via MCP gateway. Tests create, list, changes, revert, and delete at 3 workspace sizes (10, 100, 500 files).

Each operation is measured as the full round-trip: guest CLI invocation through the MCP server relay, over vsock to the host gateway, filesystem operation, and response back to the guest.

The host gateway-side latency is also recorded in the session database (`mcp_calls.duration_ms`). Use `just inspect-session` to see the MCP tool usage breakdown after a benchmark run.

Typical gateway-side latencies (macOS, APFS clonefile):

| Operation | Gateway (ms) |
|-----------|-------------|
| create | 3-5 |
| list | 2-4 |
| changes | 3-5 |
| revert | 1-2 |
| delete | 10-20 |

## JSON output

All benchmarks emit structured JSON to stdout for machine consumption:

```json
{
  "version": "0.3.0",
  "timestamp": 1711561234.5,
  "hostname": "capsem",
  "disk": { ... },
  "rootfs": { ... },
  "startup": { ... },
  "http": { ... },
  "throughput": { ... },
  "snapshot": {
    "10_files": { "create_ms": 380, "list_ms": 370, ... },
    "100_files": { ... },
    "500_files": { ... }
  }
}
```
