---
title: Performance Results
description: Reference benchmark results for Capsem VM boot time, disk I/O, CLI startup, network, and snapshot operations.
sidebar:
  order: 1
---

Reference results from local benchmark artifacts. Guest measurements come from
`capsem-bench` 0.3.0; lifecycle, fork, host-native, Criterion, and
VM-originated Security Engine measurements are host-side benchmark artifacts.
The current Linux artifact set was refreshed on 2026-05-29 with
`just benchmark`. Numbers vary with host load, network path, and cache state.
Performance runs should be recorded with `just benchmark` so artifacts include
architecture, host metadata, git commit, and an optional stable run id.

## Boot time

Total time from VM start to shell ready: **~580ms**.

| Stage | Duration | Description |
|-------|----------|-------------|
| squashfs | 10ms | Mount compressed rootfs from virtio block device |
| virtiofs | <1ms | Mount VirtioFS shared directory |
| overlayfs | 80ms | Create ext4 loopback overlay (format + mount) |
| workspace | <1ms | Bind-mount /root from VirtioFS |
| network | 210ms | Configure dummy0 and iptables DNS/HTTPS redirect rules |
| dns_proxy | tracked separately | Start UDP/TCP DNS bridge to host vsock:5007 |
| net_proxy | 100ms | Start TCP-to-vsock HTTPS proxy |
| deploy | 10ms | Copy tools from initrd to rootfs |
| venv | 170ms | Create Python virtualenv (via uv) |
| agent_start | <1ms | Launch PTY agent, connect vsock |
| **Total** | **~580ms** | |

The diagnostic suite enforces boot time stays under 1 second. The two heaviest stages are network setup (iptables rule installation) and venv creation.

## Disk I/O

Scratch disk performance in the writable scratch/system lane (`/var/tmp`).
Test size: 256MB. Workspace/VirtioFS performance for `/root` is tracked by the
storage split benchmark.

| Test | Throughput | IOPS | Duration |
|------|-----------|------|----------|
| Sequential write (1MB blocks) | 174.1 MB/s | - | 1,470.2ms |
| Sequential read (1MB blocks) | 809.1 MB/s | - | 316.4ms |
| Random 4K write (fdatasync) | 9.3 MB/s | 2,374 | 4,212.9ms |
| Random 4K read | 2,200.4 MB/s | 563,314 | 17.8ms |

Sequential I/O reflects the active host filesystem and hypervisor backend. Random write IOPS are limited by per-write `fdatasync` -- this reflects the worst case for database-style workloads.

## Rootfs reads

Read-only squashfs rootfs where binaries and libraries live.

| Test | Detail | Throughput | IOPS | Duration |
|------|--------|-----------|------|----------|
| Sequential read (1MB) | Claude binary (228.5MB) | 189.1 MB/s | - | 1,208.6ms |
| Random 4K read | 2,612 files sampled | 6.3 MB/s | 1,620 | 3,086.0ms |
| Large binary cold reads | 3 binaries, 668.8MB total | 188.1 MB/s | - | 3,556.6ms |
| Small JS/package reads | 113 files sampled | 671.0 MB/s | 79,606 ops/s | 62.8ms |
| Metadata stat walk | 6,573 entries | - | 42,384 stats/s | 155.1ms |

Squashfs decompression adds overhead compared to the scratch disk. Random reads across many small files show the cost of decompression + inode lookup on a compressed filesystem.

## CLI cold-start latency

Wall-clock time to run `<cli> --version` with page cache dropped (3 runs, best/mean/worst).

| CLI | Min | Mean | Max |
|-----|-----|------|-----|
| python3 | 31.1ms | 36.6ms | 47.1ms |
| node | 295.7ms | 298.1ms | 299.6ms |
| claude | 1,287.4ms | 1,388.7ms | 1,439.6ms |
| gemini | 2,976.6ms | 3,092.2ms | 3,279.6ms |
| codex | 817.1ms | 835.6ms | 872.5ms |

Python starts near-instantly. Node-based CLIs and native agent CLIs generally start in the low hundreds of milliseconds.

## HTTP throughput

50 GET requests to `https://www.google.com/` with concurrency 5, routed through the MITM proxy.

| Metric | Value |
|--------|-------|
| Requests | 50/50 |
| Requests/sec | 61.4 |
| Transfer | 3.8MB |
| Total duration | 814.2ms |

| Latency percentile | Value |
|--------------------|-------|
| min | 47.4ms |
| p50 | 54.3ms |
| p95 | 281.5ms |
| p99 | 287.0ms |
| max | 290.0ms |

Latency includes the full path: guest -> net-proxy -> vsock -> host MITM proxy -> TLS termination -> internet -> re-encryption -> response. The tail mostly reflects upstream internet latency and TLS/session setup.

## Proxy throughput

Reference file download through the MITM proxy.

| Metric | Value |
|--------|-------|
| Downloaded | 9.98MB |
| Duration | 0.532s |
| Throughput | 17.89 MB/s |

This is the sustained bandwidth ceiling for the proxy pipeline (TLS termination + body inspection + re-encryption). Actual throughput varies with internet connection speed.

## Snapshot operations

End-to-end latency for snapshot operations via the guest MCP endpoint at 3 workspace sizes. Each operation is a full round-trip: guest CLI -> framed vsock -> host endpoint -> host filesystem -> response.

### 10 files

| Operation | Latency |
|-----------|---------|
| create | 2,945.6ms |
| list | 935.2ms |
| changes | 934.1ms |
| revert | 933.5ms |
| delete | 945.3ms |

### 100 files

| Operation | Latency |
|-----------|---------|
| create | 1,052.9ms |
| list | 946.4ms |
| changes | 946.7ms |
| revert | 943.5ms |
| delete | 974.2ms |

### 500 files

| Operation | Latency |
|-----------|---------|
| create | 1,030.6ms |
| list | 957.8ms |
| changes | 995.8ms |
| revert | 956.4ms |
| delete | 980.3ms |

The 10-file `create` is slower than 100/500 because it includes the first MCP handshake (JSON-RPC initialize). Subsequent operations reuse the connection. List and changes scale modestly with file count. The host gateway-side latency is typically 3-20ms -- the rest is vsock + MCP protocol overhead.

## VM lifecycle (host-side)

Host-side latency for individual VM operations. Measured over 3 provision/exec/delete cycles on the same service instance.

| Operation | Min | Mean | Max | Description |
|-----------|-----|------|-----|-------------|
| provision | 2,238.2ms | 2,240.3ms | 2,243.4ms | Create and boot a temporary VM |
| exec_ready | 23.3ms | 25.0ms | 28.3ms | First ready check after provisioning |
| exec | 23.0ms | 23.7ms | 24.2ms | Simple `echo ok` on running VM |
| delete | 166.8ms | 167.2ms | 167.5ms | VM teardown request |
| **total** | **2,454.2ms** | **2,456.2ms** | **2,457.3ms** | |

Provision includes the boot path, so it carries the bulk of lifecycle latency. Exec and ready checks are low-latency once the VM is running.

Run: `uv run pytest tests/capsem-serial/test_lifecycle_benchmark.py::test_lifecycle_benchmark -xvs`

## Fork (host-side)

Host-side latency for fork (image creation) and boot-from-image. Measured over 3 cycles: create VM, install jq, write workspace files, fork, boot from image, verify data survived.

| Metric | Min | Mean | Max | Gate | Description |
|--------|-----|------|-----|------|-------------|
| fork | 114.6ms | 115.1ms | 115.4ms | 500ms | Reflink/sparse-preserving copy of rootfs overlay + workspace |
| image_size | 91.8MB | 101.1MB | 105.8MB | 128MB | Actual disk (blocks), not logical sparse size |
| boot_provision | 1,485.6ms | 1,514.1ms | 1,529.4ms | 1,200ms | Clone image into new session + boot |
| boot_ready | 26.1ms | 29.8ms | 35.3ms | 1,200ms | First ready check after provisioning |

Fork is fast because the backend uses copy-on-write or sparse-preserving copy paths where available. Image size reports actual allocated blocks, not the logical sparse file size. Both rootfs overlay changes (installed packages) and workspace files (`/root/`) survive fork.

**Regression gates**: fork < 500ms, image < 16MB, packages + workspace must survive every run.

Run: `uv run pytest tests/capsem-serial/test_lifecycle_benchmark.py::test_fork_benchmark -xvs`

## Security Engine CEL microbench (host-side)

Current host-side microbenchmark artifact:
`benchmarks/security-engine/data_1.2.1779673506_x86_64_cel_microbench.json`.
Detection IR parse/lowering artifact:
`benchmarks/security-engine/data_1.2.1779673506_x86_64_security_packs_microbench.json`.

These are Rust Criterion microbenchmarks for canonical policy-context CEL paths
and Detection IR pack parsing/lowering. They are not VM-originated benchmarks
and should not be used as end-to-end latency claims.

| Benchmark | Slope |
|-----------|-------|
| Compile `http.request.host.contains("google")` | 18.1us |
| Compile full HTTP policy | 109.0us |
| Evaluate `http.request.host.contains("google")` | 39.8us |
| Evaluate `http.request.header("authorization").exists()` | 46.8us |
| Evaluate full HTTP policy | 66.1us |
| Evaluate full HTTP policy as last match across 100 rules | 3.47ms |
| Detection finding for full HTTP policy | 66.5us |
| Detection finding as last match across 100 rules | 3.46ms |
| Dedupe 100 backtest rows / 100 unique signatures | 67.1us |
| Dedupe 1,000 backtest rows / 100 unique signatures | 584.4us |
| Runtime registry install/update of one rule | 202.6ns |
| Runtime registry projection of 100 enabled rules | 23.6us |
| Runtime projection and compile of 100 enforcement rules | 512.3us |
| Runtime projection and compile of 100 detection rules | 534.4us |
| Rebuild engine from 100 enforcement and 100 detection rules | 1.05ms |
| Update one existing rule and rebuild 100-rule plan | 688.8us |
| Project `SecurityEvent` to `PolicyContext` | 903.1ns |
| Project and serialize `PolicyContext` | 6.8us |
| Native Rust lookup for equivalent HTTP policy | 40.4ns |
| Parse and validate Detection IR Google-secret fixture | 409.9us |
| Lower Detection IR Google-secret fixture to CEL rules | 1.5us |
| Lower 100 Detection IR HTTP rules to CEL rules | 190.2us |
| Lower and compile 100 Detection IR HTTP rules | 7.2ms |

Run:

```bash
just benchmark
```

## Security Engine process enforcement (VM-originated)

Current VM-originated benchmark artifact:
`benchmarks/security-engine/data_1.2.1779673506_x86_64_process_enforcement.json`.

This host-side serial benchmark runs a live service and VM, installs a runtime
CEL rule that blocks shell process exec, sends eight blocked exec requests, and
verifies the response, runtime match counters, canonical `session.db` security
events, and `logs` exposure.

| Metric | Value |
|--------|-------|
| Runs | 8 |
| Gate | 750ms mean |
| Min blocked exec latency | 13.758ms |
| Mean blocked exec latency | 14.308ms |
| Median blocked exec latency | 14.329ms |
| p95 blocked exec latency | 14.759ms |
| p99 blocked exec latency | 14.759ms |
| Max blocked exec latency | 14.759ms |
| Runtime matches | 8 |
| Session DB security events | 8 |

Run:

```bash
uv run pytest tests/capsem-serial/test_security_engine_benchmark.py -xvs
```

## Security Engine HTTP request enforcement (VM-originated)

Current network-transport benchmark artifact:
`benchmarks/security-engine/data_1.2.1779673506_x86_64_http_request_enforcement.json`.

This host-side serial benchmark runs a live service and VM, installs a runtime
CEL rule that blocks a specific HTTPS request before upstream dispatch, warms
the path once, then runs a guest curl loop and verifies the block responses,
runtime match counters, canonical `session.db` security events, and `logs`
exposure. It also runs a persistent TLS keep-alive client over the same
connection to prove repeated block decisions stay logged and avoid per-request
TLS setup in the hot path.

The wall-clock metric includes spawning curl in the guest. The
`time_starttransfer` metric is curl's first-byte timing for the blocked
response and is the better proxy for transport plus Security Engine response
latency. The phase deltas show most first-byte time is TLS/MITM appconnect;
the post-pretransfer server-first-byte slice, which includes request dispatch,
Security Engine evaluation, synthetic 403 generation, and first-byte delivery,
is below 1ms on this run.

| Metric | Value |
|--------|-------|
| Runs | 8 |
| Warmup runs | 1 |
| Gate | 1,000ms mean |
| Mean wall-clock blocked request | 19.220ms |
| Median wall-clock blocked request | 18.751ms |
| p95 wall-clock blocked request | 22.104ms |
| Mean `time_starttransfer` | 9.523ms |
| Median `time_starttransfer` | 9.217ms |
| p95 `time_starttransfer` | 11.818ms |
| Mean DNS | 2.615ms |
| Mean TCP connect | 2.718ms |
| Mean TLS appconnect | 7.675ms |
| Runtime matches | 17 |
| Session DB security events | 17 |

Run:

```bash
uv run pytest tests/capsem-serial/test_security_engine_benchmark.py::test_http_request_enforcement_benchmark_records_vm_originated_path -xvs
```

## Security Engine DNS request enforcement (VM-originated)

Current DNS-transport benchmark artifact:
`benchmarks/security-engine/data_1.2.1779673506_x86_64_dns_request_enforcement.json`.

This host-side serial benchmark runs a live service and VM, installs a runtime
CEL rule that blocks one DNS qname, triggers repeated guest resolver lookups,
and verifies NXDOMAIN-style failure, runtime match counters, canonical
`session.db` security events, `dns_events` policy fields, and `logs` qname
attribution.

| Metric | Value |
|--------|-------|
| Runs | 8 |
| Gate | 1,000ms mean |
| Min blocked DNS lookup | 1.221ms |
| Mean blocked DNS lookup | 2.305ms |
| Median blocked DNS lookup | 1.566ms |
| p95 blocked DNS lookup | 7.655ms |
| p99 blocked DNS lookup | 7.655ms |
| Max blocked DNS lookup | 7.655ms |
| Runtime matches | 16 |
| Session DB security events | 16 |
| Session DB DNS events | 16 |

Run:

```bash
uv run pytest tests/capsem-serial/test_security_engine_benchmark.py::test_dns_request_enforcement_benchmark_records_vm_originated_path -xvs
```

## Security Engine MCP request enforcement (VM-originated)

Current framed-MCP benchmark artifact:
`benchmarks/security-engine/data_1.2.1779673506_x86_64_mcp_request_enforcement.json`.

This host-side serial benchmark runs a live service and VM, installs a runtime
CEL rule that blocks the guest `local__echo` MCP tool, sends repeated
`tools/call` requests through `/run/capsem-mcp-server`, and verifies JSON-RPC
denial, runtime match counters, canonical `session.db` security events,
`mcp_calls` policy fields, and `logs` server/tool attribution.

| Metric | Value |
|--------|-------|
| Runs | 8 |
| Gate | 1,000ms mean |
| Min blocked MCP request | 0.846ms |
| Mean blocked MCP request | 1.173ms |
| Median blocked MCP request | 1.026ms |
| p95 blocked MCP request | 2.270ms |
| p99 blocked MCP request | 2.270ms |
| Max blocked MCP request | 2.270ms |
| Runtime matches | 8 |
| Session DB security events | 8 |
| Session DB MCP calls | 8 |

Run:

```bash
uv run pytest tests/capsem-serial/test_security_engine_benchmark.py::test_mcp_request_enforcement_benchmark_records_vm_originated_path -xvs
```

## Test environment

| Component | Version |
|-----------|---------|
| Host | Linux x86_64, Intel Xeon @ 2.80GHz, 16 logical CPUs, 62.79GB RAM |
| Capsem | 1.2.1779673506 benchmark artifact |
| Guest kernel | Linux 6.x (custom allnoconfig) |
| Storage | KVM/VirtioFS workspace, ext4 host backing |
| Python | 3.x (rootfs) |
| Node | v22.x (rootfs) |

## Reproducing

```bash
just benchmark

# Optional named artifact run
CAPSEM_BENCHMARK_RUN_ID=rc1 just benchmark
```

Results are displayed as rich tables in the terminal. JSON output is saved to
`/tmp/capsem-benchmark.json` inside the VM and archived under `benchmarks/`.
Set `CAPSEM_BENCHMARK_OUTPUT_DIR` to write artifacts somewhere else during
exploratory runs.
