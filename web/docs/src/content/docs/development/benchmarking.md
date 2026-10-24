---
title: Benchmarking
description: How to run, interpret, and extend the capsem-bench performance benchmarking tool.
sidebar:
  order: 30
---

Capsem's benchmark control plane is the Rust `capsem-bench-rs` binary. The gate
runs small collectors that emit raw samples; the binary alone computes
statistics and records versioned results in
`cache/target/tests/benchmarks/benchmarks.db`. Host routes, Criterion targets,
VM lifecycle, protocol, storage, and load tests therefore share one schema and
comparison policy.

## Running benchmarks

```bash
just bench-quick                    # Host-only development loop; never evidence
just bench                          # Every dimension with a collector
just bench routes criterion         # Selected dimensions
just bench-slow routes              # Rebuild stale VM assets, then measure
just bench-report                   # Subjects, inputs, and stored trends
```

Use `capsem-bench-rs doctor` when a number is surprising. A measurement from an
unqualified or busy machine remains diagnostic data.

### Before and after comparisons

Performance claims require two unchanged baseline runs and two candidate runs.
`capsem-bench-rs compare` combines the latest two authoritative runs from each
store by default, then judges every metric against observed baseline variance
and the configured minimum time resolution. Dimension and architecture must
match. A quick run is recorded for visibility but is never selected as
evidence.

Low single-digit route-latency movement is normally scheduler noise. The
comparison keeps such movement visible as `tiny` or `noisy`; only a material
change outside the baseline operating envelope is a regression. Route health
ceilings catch gross drift and are not evidence of a comparative speedup.

### Control API routes (`routes`)

The route collector measures each configured read-only route directly over the
service UDS and through the authenticated TCP gateway. It records warm and cold
sequential latency, then drives 512 requests through 16 kept-alive concurrent
clients and records per-request latency, aggregate RPS, and CPU for every
participating process. The quick lane uses 64 requests but is diagnostic only.

The route inventory is derived from the registered service and gateway route
tables. Every endpoint must be classified as measured, explicitly unmeasured,
or internal. The generic load lane excludes mutations and routes that require
external network state; their lifecycle or protocol owner supplies the safe
fixture instead. This means `routes` provides RPS for every route in its
measured class, rather than claiming that arbitrary mutations are safe to
replay as a load test.

## Disabled warm operations in `routes`

Warm suspend/restore is disabled for 0.7.1 pending [#378](https://github.com/google/capsem/issues/378).
The active pause case measures an actual 501 refusal while the original process,
boot, generation and session remain unchanged. The resume case measures 501 for
an owned synthetic saved-session fixture and verifies its checkpoint bytes remain
intact. These are unsupported-operation measurements, not native checkpoint or
warm-continuity proof. HTTP and verified-effect clocks remain separate. Original
warm identity regressions are retained for #378; they must pass before warm
suspend is re-enabled. Cold start is never reported as warm success.

## Managed restart (`restart`)

`capsem-gate bench --commit <exact-sha> restart` uses the existing disposable
native supervisor owner. Ordinary route measurements remain confined. The
default complete benchmark plan runs the confined selection with
`--exclude restart`, then a separate native restart-only invocation of the same
Rust runner. A complete ALL result requires the union of both sets of recording
IDs, with identical source and configuration; the ordinary invocation alone is
incomplete. Explicit mixed selections are partitioned the same way. Quick
diagnostics exclude restart.

The finite case restarts an isolated service with a known stopped workspace.
HTTP 202 measures admission only. Separate raw clocks report observed original
process retirement, new authenticated readiness, reconnection and verified
registry/workspace preservation. Old-token rejection is checked on the new
listener. CPU and sampled resident peaks follow only the owning fixture's
stamped process roots, including replacements, through the existing sampler;
these are not exact startup peaks. The case does not boot a VM, measure warm
restore or certify live workload recovery. Native execution and two baseline /
two candidate fitness-qualified runs are required before any performance claim.

## Boot timing

Boot timing is measured independently from `capsem-bench`. The guest init script (`capsem-init`) records the wall-clock duration of each boot stage using `/proc/uptime`. The PTY agent sends these measurements to the host over the vsock control channel, where they are displayed as an inline table with a proportional bar chart.

### Measured stages

| Stage | What happens |
|-------|-------------|
| `rootfs` | Mount the compressed read-only rootfs from the virtio block device |
| `virtiofs` | Mount the VirtioFS shared directory from the host |
| `overlayfs` | Create the overlay filesystem (ext4 loopback upper + EROFS lower) |
| `workspace` | Bind-mount `/root` from the VirtioFS workspace |
| `network` | Configure dummy0 interface and iptables DNS/HTTPS redirect rules |
| `dns_proxy` | Start capsem-dns-proxy and bridge DNS to host vsock:5007 |
| `net_proxy` | Start the TCP-to-vsock proxy for HTTPS interception |
| `deploy` | Copy MCP server, capsem-doctor, capsem-bench, and diagnostics from initrd |
| `venv` | Create the Python virtualenv (uses `uv` for speed) |
| `agent_start` | Launch the PTY agent and connect vsock ports |

### Invariant

The diagnostic suite enforces that every attributable init stage stays at or
below 500ms (`test_environment.py::test_boot_stages_within_budget`). Aggregate
first-boot time is still reported, but it is not a deterministic gate on shared
CI hosts because guest wall time includes periods when the host deschedules the
VM. Runtime diagnostics independently prove that `uv` exists and that the
default Python virtualenv is active, so the slower `python3 -m venv` fallback
cannot pass silently.

## Benchmark categories

### Disk I/O (`disk`)

Measures scratch disk performance in `/root` (VirtioFS-backed workspace).

| Test | Method | Metric |
|------|--------|--------|
| Sequential write | Write 256MB in 1MB blocks, `fdatasync` at end | Throughput (MB/s) |
| Sequential read | Read 256MB in 1MB blocks after `drop_caches` | Throughput (MB/s) |
| Random 4K write | 10,000 random `pwrite` calls on 64MB file, `fdatasync` per write | IOPS, throughput |
| Random 4K read | 10,000 random `pread` calls on 64MB file after `drop_caches` | IOPS, throughput |

Write test size is configurable via `CAPSEM_BENCH_SIZE_MB` (default: 256).

### Rootfs reads (`rootfs`)

Measures read performance on the compressed rootfs where binaries and libraries live.

| Test | Method | Metric |
|------|--------|--------|
| Sequential read | Read the largest file in `/usr/bin` and `/usr/lib` in 1MB blocks | Throughput (MB/s) |
| Random 4K read | 5,000 random `pread` calls across all rootfs files (>4KB) | IOPS, throughput |
| Large binary reads | Cold/warm reads of the largest binaries | Throughput (MB/s), duration |
| Small package reads | Whole-file reads of small JS/package files | Duration, throughput |
| Metadata scan | Repeated `stat` calls over rootfs files | Stat/sec, latency |

### Storage split (`storage`)

Records where storage time goes across rootfs, workspace, tmpfs, overlay, and
kernel queues. This is the release diagnostic for EROFS/LZ4HC and Linux KVM
storage tuning.

| Area | What it records |
|------|-----------------|
| Kernel context | cmdline, block queue knobs, FUSE backpressure knobs, known host queue sizes |
| Mounts | Parsed `/proc/self/mountinfo` with filesystem type/source/options |
| Rootfs backing | overlay lower/upper/workdir and read-only image metadata |
| Writable paths | sequential/random I/O profiles for `/root`, `/tmp`, `/var/tmp`, `/var/log`, `/run` |

Useful environment overrides:

- `CAPSEM_STORAGE_BENCH_PATHS`: colon-separated writable paths to profile.
- `CAPSEM_STORAGE_BENCH_SIZE_MB`: storage split write size.
- `CAPSEM_STORAGE_IO_PROFILE_SIZE_MB`: sequential profile file size.
- `CAPSEM_STORAGE_IO_PROFILE_RANDOM_OPS`: random I/O operation count.

### CLI cold-start (`startup`)

Measures wall-clock time to run `<cli> --version` with page cache dropped between runs. Each command is timed 3 times.

| Command | What it tests |
|---------|--------------|
| `python3 --version` | CPython interpreter startup |
| `node --version` | Node.js runtime startup |
| `claude --version` | Claude Code CLI (Node-based) |
| `gemini --version` | Gemini CLI (Node-based) |
| `codex --version` | Codex CLI (native binary + Node) |

### HTTP (`http`)

Measures HTTP throughput through the MITM proxy using concurrent GET requests.

- **Default**: skipped unless `CAPSEM_MOCK_SERVER_BASE_URL` is set.
- **Local release proof**: set `CAPSEM_MOCK_SERVER_BASE_URL` to the
  host-side `capsem-mock-server` base URL; `http` targets `/tiny`.
- **Custom**: `capsem-bench http <URL> <N> <C>`
- **Reports**: successful/failed count, requests/sec, latency percentiles (p50, p95, p99, min, max)

Each worker thread uses a persistent `requests.Session`. Latency includes the
full round-trip: guest -> net-proxy -> vsock -> host MITM proxy -> local debug
upstream -> response back.

### Proxy throughput (`throughput`)

Downloads a deterministic 10 MB local fixture through the MITM proxy and
reports end-to-end throughput when `CAPSEM_MOCK_SERVER_BASE_URL` is set.
Public throughput is explicit opt-in only via
`CAPSEM_BENCH_ALLOW_PUBLIC_NETWORK=1`; it is not release proof.

### Load tests (`mitm-load`, `mcp-load`, `dns-load`)

These modes are opt-in because they stress hot paths more aggressively than the default `all` suite.

| Mode | What it exercises |
|------|-------------------|
| `mitm-load` | Concurrent HTTPS requests through the MITM proxy |
| `mcp-load` | Guest MCP framed transport and host endpoint dispatch |
| `dns-load` | DNS redirect, capsem-dns-proxy, host DNS policy, and resolver path |

Release benchmark proof must use local fixtures. Public-network HTTP,
throughput, model, or DNS numbers are debugging data only and cannot close the
release gate.

All load tests use the same concurrency and duration contract:

- `CAPSEM_BENCH_CONCURRENCY`: one value (`64`) or a comma-separated sweep (`1,10,50,200`).
- `CAPSEM_BENCH_DURATION_S`: seconds per concurrency level for duration-based load tests.

A load run is valid only when every request succeeds at every configured
concurrency level. One transport, protocol, DNS, or HTTP error rejects the run
before its latency or throughput can become comparison data. Successful runs
record `errors = 0` and `error_ratio = 0` for each level as the proof that the
reported samples cover the complete load rather than only the successful tail.

### Multi-VM MITM scaling (`mitm-scale`)

```bash
just bench mitm-scale
```

This host-side dimension creates a fresh identical cohort at each configured
VM count, starts one fixed-concurrency keep-alive HTTPS load in every VM, and
measures all VMs at the same time. Fresh cohorts keep memory and lifecycle
history from one level out of the next. The collector records:

- aggregate and per-VM RPS;
- per-VM p50, p95, p99, and p99.9 latency;
- PSS, USS, RSS, CPU time, threads, descriptors, and I/O for the Capsem process
  tree;
- the same resource pressure for the mock upstream in a separate namespace;
- host physical-core and logical-CPU counts; and
- request errors plus proxy traffic-grant refusals.

Any request error, starved client loop, or hidden proxy refusal fails the
dimension. Compare two unchanged runs. Interpret scaling against physical
cores first and logical CPUs second: when total Capsem CPU reaches the host
budget, rising latency or falling aggregate RPS is a host-capacity result.
Before that point it can indicate a shared lock, serialized queue, or fixture
bottleneck. Fixture CPU is reported separately so upstream saturation cannot
be mistaken for a Capsem limit.

`capsem-bench protocol` runs deterministic local mock-server scenarios: tiny
HTTP, 1 MiB body, gzip, SSE model stream, JSON model response, denied-target,
credential-shaped response, and WebSocket control frames. When
`CAPSEM_MOCK_SERVER_BASE_URL` is set, `capsem-bench all` includes the same
protocol group after the broad disk/rootfs/storage/startup/http/throughput
suite.

- `CAPSEM_BENCH_TOTAL_REQUESTS`: requests per selected local MITM scenario.
- `CAPSEM_BENCH_SCENARIOS`: comma-separated local MITM scenario names, for example `model_json_response,credential_response`.

The same values are available as CLI arguments:

```bash
CAPSEM_MOCK_SERVER_BASE_URL=http://127.0.0.1:3713 CAPSEM_BENCH_TOTAL_REQUESTS=50000 CAPSEM_BENCH_CONCURRENCY=64 CAPSEM_BENCH_SCENARIOS=model_json_response,credential_response capsem-bench protocol
capsem-bench mcp-load 64 5
capsem-bench dns-load 64 5
```

Host-side benchmark artifacts can be validated and rendered with:

```bash
uv run --project build_system --frozen build_system/scripts/build/benchmark_report.py benchmarks/baselines/mcp-load/baseline.json benchmarks/baselines/dns-load/baseline.json benchmarks/baselines/mock-server-protocol/control_host_direct_c64_model_credential_1.0.1780954707_arm64.json
uv run --project build_system --frozen --with matplotlib build_system/scripts/build/benchmark_report.py benchmarks/baselines/mcp-load/baseline.json benchmarks/baselines/dns-load/baseline.json benchmarks/baselines/mock-server-protocol/control_host_direct_c64_model_credential_1.0.1780954707_arm64.json --plot benchmarks/baselines/load_baseline_report.png
```

## JSON output

All benchmarks save structured JSON to `/tmp/capsem-benchmark.json` inside the VM:

```json
{
  "version": "0.3.0",
  "timestamp": 1711561234.5,
  "hostname": "capsem",
  "disk": { "seq_write": { "throughput_mbps": 1180, ... }, ... },
  "rootfs": { ... },
  "startup": { "commands": { "python3": { "mean_ms": 9.0 }, ... } },
  "http": { "requests_per_sec": 58, "latency_ms": { "p50": 67, ... } },
  "throughput": { "throughput_mbps": 34.3, ... },
  "storage": { "kernel": { ... }, "rootfs": { ... }, "writable": { ... } },
  "dns_load": { "qname": "api.openai.com", "levels": [...] }
}
```

## Adding a new benchmark

1. Create a new module in `guest/artifacts/capsem_bench/` (e.g., `mytest.py`) with a `mytest_bench()` function that returns a dict and prints a Rich table to stderr
2. Add the mode name to `VALID_MODES` in `capsem_bench/__main__.py`
3. Wire it into `main()` with the `if mode in ("name", "all"):` pattern (lazy import)
4. Update the `dev-benchmark` skill and this page
