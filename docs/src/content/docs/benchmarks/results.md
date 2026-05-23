---
title: Performance Results
description: Reference benchmark results for Capsem VM boot time, disk I/O, CLI startup, network, and snapshot operations.
sidebar:
  order: 1
---

Reference results from the latest local benchmark artifacts recorded on 2026-05-03. Guest measurements come from `capsem-bench` 0.3.0; lifecycle and fork measurements are host-side benchmark runs. Numbers vary with host load, network path, and cache state.

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

Scratch disk performance on the VirtioFS-backed workspace (`/root`). Test size: 256MB.

| Test | Throughput | IOPS | Duration |
|------|-----------|------|----------|
| Sequential write (1MB blocks) | 1,854 MB/s | - | 138ms |
| Sequential read (1MB blocks) | 3,754 MB/s | - | 68ms |
| Random 4K write (fdatasync) | 33 MB/s | 8,353 | 1,197ms |
| Random 4K read | 279 MB/s | 71,440 | 140ms |

Sequential I/O benefits from VirtioFS pass-through to APFS. Random write IOPS are limited by per-write `fdatasync` -- this reflects the worst case for database-style workloads.

## Rootfs reads

Read-only squashfs rootfs where binaries and libraries live.

| Test | Detail | Throughput | IOPS | Duration |
|------|--------|-----------|------|----------|
| Sequential read (1MB) | codex binary (193MB) | 693 MB/s | - | 266ms |
| Random 4K read | 2,588 files sampled | 38 MB/s | 9,783 | 511ms |

Squashfs decompression adds overhead compared to the scratch disk. Random reads across many small files show the cost of decompression + inode lookup on a compressed filesystem.

## CLI cold-start latency

Wall-clock time to run `<cli> --version` with page cache dropped (3 runs, best/mean/worst).

| CLI | Min | Mean | Max |
|-----|-----|------|-----|
| python3 | 7ms | 9ms | 11ms |
| node | 126ms | 128ms | 132ms |
| claude | 335ms | 337ms | 340ms |
| gemini | 594ms | 599ms | 605ms |
| codex | 293ms | 293ms | 293ms |

Python starts near-instantly. Node-based CLIs and native agent CLIs generally start in the low hundreds of milliseconds.

## HTTP throughput

50 GET requests to `https://www.google.com/` with concurrency 5, routed through the MITM proxy.

| Metric | Value |
|--------|-------|
| Requests | 50/50 |
| Requests/sec | 19.6 |
| Transfer | 3.8MB |
| Total duration | 2,557ms |

| Latency percentile | Value |
|--------------------|-------|
| min | 107ms |
| p50 | 162ms |
| p95 | 659ms |
| p99 | 713ms |
| max | 732ms |

Latency includes the full path: guest -> net-proxy -> vsock -> host MITM proxy -> TLS termination -> internet -> re-encryption -> response. The tail mostly reflects upstream internet latency and TLS/session setup.

## Proxy throughput

Reference file download through the MITM proxy.

| Metric | Value |
|--------|-------|
| Downloaded | 9.98MB |
| Duration | 4.56s |
| Throughput | 2.09 MB/s |

This is the sustained bandwidth ceiling for the proxy pipeline (TLS termination + body inspection + re-encryption). Actual throughput varies with internet connection speed.

## Snapshot operations

End-to-end latency for snapshot operations via the guest MCP endpoint at 3 workspace sizes. Each operation is a full round-trip: guest CLI -> framed vsock -> host endpoint -> APFS filesystem -> response.

### 10 files

| Operation | Latency |
|-----------|---------|
| create | 1,217ms |
| list | 514ms |
| changes | 463ms |
| revert | 457ms |
| delete | 444ms |

### 100 files

| Operation | Latency |
|-----------|---------|
| create | 507ms |
| list | 463ms |
| changes | 439ms |
| revert | 417ms |
| delete | 370ms |

### 500 files

| Operation | Latency |
|-----------|---------|
| create | 377ms |
| list | 372ms |
| changes | 402ms |
| revert | 420ms |
| delete | 430ms |

The 10-file `create` is slower than 100/500 because it includes the first MCP handshake (JSON-RPC initialize). Subsequent operations reuse the connection. List and changes scale modestly with file count. The host gateway-side latency is typically 3-20ms -- the rest is vsock + MCP protocol overhead.

## VM lifecycle (host-side)

Host-side latency for individual VM operations. Measured over 3 provision/exec/delete cycles on the same service instance.

| Operation | Min | Mean | Max | Description |
|-----------|-----|------|-----|-------------|
| provision | 895ms | 931ms | 951ms | Create and boot a temporary VM |
| exec_ready | 11.5ms | 12.1ms | 12.9ms | First ready check after provisioning |
| exec | 10.7ms | 10.9ms | 11.3ms | Simple `echo ok` on running VM |
| delete | 60.1ms | 60.6ms | 61.5ms | VM teardown request |
| **total** | **980ms** | **1,015ms** | **1,033ms** | |

Provision includes the boot path, so it carries the bulk of lifecycle latency. Exec and ready checks are low-latency once the VM is running.

Run: `uv run pytest tests/capsem-serial/test_lifecycle_benchmark.py::test_lifecycle_benchmark -xvs`

## Fork (host-side)

Host-side latency for fork (image creation) and boot-from-image. Measured over 3 cycles: create VM, install jq, write workspace files, fork, boot from image, verify data survived.

| Metric | Min | Mean | Max | Gate | Description |
|--------|-----|------|-----|------|-------------|
| fork | 83ms | 88ms | 93ms | 500ms | APFS clonefile of rootfs overlay + workspace |
| image_size | 7.5MB | 7.5MB | 7.5MB | 16MB | Actual disk (blocks), not logical sparse size |
| boot_provision | 744ms | 747ms | 752ms | 1,200ms | Clone image into new session + boot |
| boot_ready | 11ms | 11ms | 12ms | 1,200ms | First ready check after provisioning |

Fork is fast because APFS `clonefile()` is copy-on-write -- no actual data copying. Image size reports actual allocated blocks, not the logical 2GB sparse file size. Both rootfs overlay changes (installed packages) and workspace files (`/root/`) survive fork.

**Regression gates**: fork < 500ms, image < 16MB, packages + workspace must survive every run.

Run: `uv run pytest tests/capsem-serial/test_lifecycle_benchmark.py::test_fork_benchmark -xvs`

## Security Engine CEL microbench (host-side)

First S08d host-side microbenchmark artifact:
`benchmarks/security-engine/data_1.1.1778860037_arm64_cel_microbench.json`.

This is a Rust Criterion microbenchmark for canonical policy-context CEL paths.
It is not a VM-originated benchmark and should not be used as an end-to-end
latency claim.

| Benchmark | Slope |
|-----------|-------|
| Compile `http.request.host.contains("google")` | 8.7us |
| Compile full HTTP policy | 44.0us |
| Evaluate `http.request.host.contains("google")` | 14.6us |
| Evaluate `http.request.header("authorization").exists()` | 17.0us |
| Evaluate full HTTP policy | 23.7us |
| Evaluate full HTTP policy as last match across 100 rules | 1.29ms |
| Project `SecurityEvent` to `PolicyContext` | 543ns |
| Project and serialize `PolicyContext` | 2.6us |
| Native Rust lookup for equivalent HTTP policy | 12ns |

Run:

```bash
cargo bench -p capsem-security-engine --bench security_engine_cel
```

## Test environment

| Component | Version |
|-----------|---------|
| Host | Apple Silicon macOS local benchmark host |
| Capsem | 1.0 benchmark artifact |
| Guest kernel | Linux 6.x (custom allnoconfig) |
| Storage | VirtioFS mode (APFS backing) |
| Python | 3.x (rootfs) |
| Node | v22.x (rootfs) |

## Reproducing

```bash
just bench    # Run all benchmarks (~2 min)
```

Results are displayed as rich tables in the terminal. JSON output is saved to `/tmp/capsem-benchmark.json` inside the VM.
