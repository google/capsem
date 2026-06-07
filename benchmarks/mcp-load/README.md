# mcp-load baselines

Pre-rewrite baseline at `baseline.json`. Sister bench to
`benchmarks/mitm-load/baseline.json`. T5's CI gate compares against
this file: any concurrency level showing >2x p99 regression on the MCP
path fails the build.

## What this measures

`mcp-load` drives the diagnostic `local__echo` MCP tool (input text in,
same text out, zero I/O) at multiple concurrency levels. End-to-end
path:

```
Python fastmcp.Client (in guest)
  -> stdio -> /run/capsem-mcp-server (guest agent's MCP server)
  -> framed MCP over vsock:5002 -> MITM MCP endpoint (host)
  -> capsem-mcp-aggregator (host)
  -> stdio -> capsem-mcp-builtin (host subprocess)
  -> echo handler returns the text
  -> back up the chain
```

Pure protocol cost. If `mcp-load` does not scale linearly with
concurrency, there is a serialization point in the guest relay / MITM
endpoint / aggregator / server / vsock chain.

## Pre-rewrite headline numbers

| concurrency | rps    | p50_ms | p95_ms | p99_ms | p999_ms |
|------------:|-------:|-------:|-------:|-------:|--------:|
| 1           | 2162.5 | 0.4    | 0.6    | 1.1    | 2.3     |
| 10          | 3792.0 | 2.4    | 3.7    | 4.4    | 7.8     |
| 50          | 4061.4 | 12.0   | 13.9   | 17.4   | 31.9    |
| 200         | 3965.0 | 48.7   | 60.5   | 70.8   | 84.2    |

**Sub-linear scaling.** Plateaus at ~4000 rps from concurrency 10
onwards. There is a serialization point we will need to debug --
suspect candidates: stdio framing in `capsem-mcp-server`, framed vsock
single-stream, JSON-RPC dispatch in the aggregator. `mitm-load`
plateaus around ~3000 rps with worse tails (cert mint + upstream pool
contention), so the MCP path is healthier than the MITM today but both
have ceilings the redesign needs to investigate.

## Capturing the baseline

```
# Persistent VM
target/debug/capsem create --name <name> --ram 4 --cpu 2

# Run the bench
target/debug/capsem exec <name> "capsem-bench mcp-load && \
  cp /tmp/capsem-benchmark.json /root/mcp-baseline.json"

# Pull via capsem cp
target/debug/capsem cp <name>:mcp-baseline.json \
  benchmarks/mcp-load/baseline.json

# Clean up
target/debug/capsem delete <name>
```

## Schema

Per-concurrency-level row:
- `concurrency`, `duration_s`, `total_requests`, `errors`
- `rps`, `p50_ms`, `p95_ms`, `p99_ms`, `p999_ms`
- `rss_peak_mb`

Defaults: concurrency `1, 10, 50, 200`; duration `10s` per level
(override via `CAPSEM_BENCH_MCP_DURATION`); echo payload `"ping"`
(override via `CAPSEM_BENCH_MCP_PAYLOAD`).
