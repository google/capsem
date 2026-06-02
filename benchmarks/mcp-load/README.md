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

## Lanes

- `fastmcp`: guest FastMCP client through `/run/capsem-mcp-server`.
- `raw-single`: raw JSON-RPC through one guest stdio relay process.
- `raw-multiprocess`: raw JSON-RPC through four guest stdio relay processes.
- `direct-vsock`: guest benchmark process speaks framed MCP directly to host
  `vsock:5002`, bypassing guest stdio relay while keeping MCP policy,
  endpoint dispatch, response policy, telemetry, and session DB writes.
- `direct-vsock-transport`: same guest AF_VSOCK connection and frame codec,
  but a reserved diagnostic method echoes before MCP policy, endpoint
  dispatch, aggregator, or session DB writes. This lane is transport
  attribution, not a product tool-path benchmark.

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

## H09 Linux diagnostic, 2026-06-02

Focused rerun from `hypervisor-improvement` after the Linux support and H09
attribution work:

```bash
just exec "capsem-bench mcp-load && cat /tmp/capsem-benchmark.json"
```

The run completed with zero errors, so unlike the same-day `mitm-load`
diagnostic it is valid transport evidence. It does not depend on DNS or an
upstream HTTP service.

| concurrency | current rps | baseline rps | rps ratio | current p99 | baseline p99 | p99 ratio |
|------------:|------------:|-------------:|----------:|------------:|-------------:|----------:|
| 1           | 309.8       | 2162.5       | 0.143x    | 3.6 ms      | 1.1 ms       | 3.21x     |
| 10          | 761.5       | 3792.0       | 0.201x    | 15.4 ms     | 4.4 ms       | 3.48x     |
| 50          | 786.1       | 4061.4       | 0.194x    | 82.2 ms     | 17.4 ms      | 4.72x     |
| 200         | 782.7       | 3965.0       | 0.197x    | 296.6 ms    | 70.8 ms      | 4.19x     |

This fails the documented >2x p99 regression gate at every concurrency level.
Treat it as an H09 source-tracing target: guest stdio relay, framed vsock
single-stream behavior, MITM MCP endpoint parsing, aggregator dispatch,
builtin stdio round trips, and telemetry writes.

## H09 transport isolation, 2026-06-02

Focused same-run proof:

```bash
just exec "CAPSEM_BENCH_MCP_LANES=direct-vsock,direct-vsock-transport \
  CAPSEM_BENCH_MCP_DURATION=5 capsem-bench mcp-load && \
  cat /tmp/capsem-benchmark.json"
```

| concurrency | direct-vsock RPS | transport RPS | transport ratio | direct p99 | transport p99 |
|------------:|-----------------:|--------------:|----------------:|-----------:|--------------:|
| 1           | 588.0            | 3,086.6       | 5.25x           | 2.6 ms     | 0.5 ms        |
| 10          | 812.8            | 13,632.2      | 16.77x          | 13.6 ms    | 1.4 ms        |
| 50          | 806.0            | 22,003.0      | 27.30x          | 105.2 ms   | 2.8 ms        |
| 200         | 822.8            | 37,027.6      | 45.00x          | 341.1 ms   | 12.6 ms       |

The transport-only lane uses the same KVM/vhost-vsock path, host fd wrapper,
frame parser, stream tracker, and response writer as the direct tool path. This
rules out raw KVM/vsock delivery as the current ~800 RPS ceiling. The next H09
target is the real MCP policy/dispatch/telemetry path after frame parsing.

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

Top-level `tool` remains `local__echo` for the MCP tool-path lanes. The
`transport_echo_method` field names the reserved diagnostic method used only by
`direct-vsock-transport`.

Defaults: lanes `fastmcp,raw-single,raw-multiprocess,direct-vsock,direct-vsock-transport`;
concurrency `1, 10, 50, 200`; duration `10s` per level (override via
`CAPSEM_BENCH_MCP_DURATION`); echo payload `"ping"` (override via
`CAPSEM_BENCH_MCP_PAYLOAD`).
