---
title: Benchmarking
description: How to run, interpret, and extend the capsem-bench performance benchmarking tool.
sidebar:
  order: 30
---

Capsem includes `capsem-bench`, a Python benchmarking tool that runs inside the VM. It outputs rich tables to stderr for humans and saves structured JSON to `/tmp/capsem-benchmark.json` for machine consumption.

## Running benchmarks

```bash
just bench                          # In-VM, lifecycle/fork, and Security Engine benchmarks
just run "capsem-bench disk"        # Disk I/O only
just run "capsem-bench rootfs"      # Rootfs reads only
just run "capsem-bench startup"     # CLI cold-start only
just run "capsem-bench http"        # HTTP through proxy
just run "capsem-bench throughput"  # 100MB download
just run "capsem-bench snapshot"    # Snapshot operations only
just run "capsem-bench mitm-load"   # MITM proxy concurrency/load test
just run "capsem-bench mcp-load"    # Guest MCP endpoint concurrency/load test
just run "capsem-bench dns-load"    # DNS proxy concurrency/load test
cargo bench -p capsem-security-engine --bench security_engine_cel
uv run pytest tests/capsem-serial/test_security_engine_benchmark.py -xvs
just full-test                      # Full validation including benchmarks
```

## Boot timing

Boot timing is measured independently from `capsem-bench`. The guest init script (`capsem-init`) records the wall-clock duration of each boot stage using `/proc/uptime`. The PTY agent sends these measurements to the host over the vsock control channel, where they are displayed as an inline table with a proportional bar chart.

### Measured stages

| Stage | What happens |
|-------|-------------|
| `squashfs` | Mount the compressed read-only rootfs from the virtio block device |
| `virtiofs` | Mount the VirtioFS shared directory from the host |
| `overlayfs` | Create the overlay filesystem (ext4 loopback upper + squashfs lower) |
| `workspace` | Bind-mount `/root` from the VirtioFS workspace |
| `network` | Configure dummy0 interface and iptables DNS/HTTPS redirect rules |
| `dns_proxy` | Start capsem-dns-proxy and bridge DNS to host vsock:5007 |
| `net_proxy` | Start the TCP-to-vsock proxy for HTTPS interception |
| `deploy` | Copy MCP server, capsem-doctor, capsem-bench, and diagnostics from initrd |
| `venv` | Create the Python virtualenv (uses `uv` for speed) |
| `agent_start` | Launch the PTY agent and connect vsock ports |

### Invariant

The diagnostic suite enforces that total boot time stays under 1 second (`test_environment.py::test_boot_time_under_1s`). Stages exceeding 500ms are flagged as slow. The most common regression is `venv` -- if `uv` is missing from the rootfs, Python falls back to `python3 -m venv` which is ~10x slower.

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

Measures read performance on the compressed squashfs rootfs where binaries and libraries live.

| Test | Method | Metric |
|------|--------|--------|
| Sequential read | Read the largest file in `/usr/bin`, `/usr/lib`, `/opt/ai-clis` in 1MB blocks | Throughput (MB/s) |
| Random 4K read | 5,000 random `pread` calls across all rootfs files (>4KB) | IOPS, throughput |

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

- **Default**: 50 requests to `https://www.google.com/` with concurrency 5
- **Custom**: `capsem-bench http <URL> <N> <C>`
- **Reports**: successful/failed count, requests/sec, latency percentiles (p50, p95, p99, min, max)

Each worker thread uses a persistent `requests.Session`. Latency includes the full round-trip: guest -> net-proxy -> vsock -> host MITM proxy -> internet -> response back.

### Proxy throughput (`throughput`)

Downloads a ~10 MB PDF through the MITM proxy and reports end-to-end throughput.

Uses `curl -L` to download `https://cdn.elie.net/static/files/i-am-a-legend/i-am-a-legend-slides.pdf` (301-redirects to `elie.net`, so both hosts must be on the allow list). This measures the maximum sustained bandwidth the proxy pipeline can deliver, including TLS termination, body inspection, and re-encryption.

### Load tests (`mitm-load`, `mcp-load`, `dns-load`)

These modes are opt-in because they stress hot paths more aggressively than the default `all` suite.

| Mode | What it exercises |
|------|-------------------|
| `mitm-load` | Concurrent HTTPS requests through the MITM proxy |
| `mcp-load` | Guest MCP framed transport and host endpoint dispatch |
| `dns-load` | DNS redirect, capsem-dns-proxy, host DNS policy, and resolver path |

### Security Engine CEL microbenchmarks

The host-side Rust Criterion harness measures canonical Security Engine CEL
paths without booting a VM:

```bash
cargo bench -p capsem-security-engine --bench security_engine_cel
cargo bench -p capsem-core --bench security_packs
```

The S08d harness covers CEL compile time, warm enforcement evaluation,
detection evaluation, backtest evidence deduplication, runtime registry
operations, compiled-plan rebuild cost, policy-context projection/
materialization, 100-rule last-match evaluation, Detection IR parse/lowering,
and a native Rust lookup comparator for the same HTTP policy. These numbers
explain runtime hot-path and rule-pack costs; they do not replace
VM-originated benchmark artifacts. Committed host-side artifacts live under
`benchmarks/security-engine/`. The `just bench` recipe runs both Criterion
harnesses before the VM-originated security benchmark.

### Security Engine VM-originated benchmarks

The host-side serial benchmark measures the real VM-originated enforcement path
for a process security event:

```bash
uv run pytest tests/capsem-serial/test_security_engine_benchmark.py -xvs
```

The first S08d paths install runtime CEL enforcement rules, send repeated
blocked process exec, blocked HTTPS request, blocked DNS lookup, and blocked
MCP `tools/call` workloads through live VMs, assert the expected block results,
check runtime match counters, verify canonical `security_events` rows in
`session.db`, and confirm `logs` exposes the Security Engine decision with
VM/profile/user/rule attribution. DNS artifacts also verify the legacy
`dns_events` row carries the runtime policy action and qname. MCP artifacts
verify `mcp_calls` policy fields and request-id-matched server/tool log
projection. Committed artifacts are written to
`benchmarks/security-engine/`.

### Snapshot operations (`snapshot`)

End-to-end latency for snapshot operations via the guest MCP endpoint. Tests at 3 workspace sizes (10, 100, 500 files of 4KB each):

| Operation | What it does |
|-----------|-------------|
| `create` | Populate workspace, create a named snapshot via `snapshots create` |
| `list` | List all snapshots with change diffs |
| `changes` | List files changed since the last checkpoint |
| `revert` | Revert a single modified file from the snapshot |
| `delete` | Delete the snapshot |

Each operation is measured as the full round-trip: guest CLI -> MCP server (NDJSON over vsock) -> host gateway -> APFS filesystem operation -> response back to guest.

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
  "snapshot": { "10_files": { "create_ms": 879, ... }, ... },
  "dns_load": { "qname": "api.openai.com", "levels": [...] }
}
```

## Adding a new benchmark

1. Create a new module in `guest/artifacts/capsem_bench/` (e.g., `mytest.py`) with a `mytest_bench()` function that returns a dict and prints a Rich table to stderr
2. Add the mode name to `VALID_MODES` in `capsem_bench/__main__.py`
3. Wire it into `main()` with the `if mode in ("name", "all"):` pattern (lazy import)
4. Update the `dev-benchmark` skill and this page
