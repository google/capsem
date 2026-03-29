---
title: Performance Results
description: Current benchmark results for Capsem VM boot time, disk I/O, CLI startup, network, and snapshot operations.
sidebar:
  order: 1
---

Benchmark results from Capsem v0.14.6 running on Apple M4 Max (macOS 15.4). All measurements taken inside the guest VM using `capsem-bench` and boot timing instrumentation.

## Boot time

Total time from VM start to shell ready: **~580ms**.

| Stage | Duration | Description |
|-------|----------|-------------|
| squashfs | 10ms | Mount compressed rootfs from virtio block device |
| virtiofs | <1ms | Mount VirtioFS shared directory |
| overlayfs | 80ms | Create ext4 loopback overlay (format + mount) |
| workspace | <1ms | Bind-mount /root from VirtioFS |
| network | 210ms | Configure dummy0, dnsmasq, iptables rules |
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
| Sequential write (1MB blocks) | 1,180 MB/s | - | 217ms |
| Sequential read (1MB blocks) | 3,425 MB/s | - | 75ms |
| Random 4K write (fdatasync) | 33 MB/s | 8,325 | 1,201ms |
| Random 4K read | 188 MB/s | 48,070 | 208ms |

Sequential I/O benefits from VirtioFS pass-through to APFS. Random write IOPS are limited by per-write `fdatasync` -- this reflects the worst case for database-style workloads.

## Rootfs reads

Read-only squashfs rootfs where binaries and libraries live.

| Test | Detail | Throughput | IOPS | Duration |
|------|--------|-----------|------|----------|
| Sequential read (1MB) | codex binary (146MB) | 727 MB/s | - | 201ms |
| Random 4K read | 4,215 files sampled | 18 MB/s | 4,704 | 1,063ms |

Squashfs decompression adds overhead compared to the scratch disk. Random reads across many small files show the cost of decompression + inode lookup on a compressed filesystem.

## CLI cold-start latency

Wall-clock time to run `<cli> --version` with page cache dropped (3 runs, best/mean/worst).

| CLI | Min | Mean | Max |
|-----|-----|------|-----|
| python3 | 8ms | 9ms | 11ms |
| node | 136ms | 138ms | 139ms |
| claude | 291ms | 310ms | 346ms |
| gemini | 1,385ms | 1,386ms | 1,389ms |
| codex | 284ms | 288ms | 291ms |

Python starts near-instantly. Node-based CLIs (claude, codex) take ~300ms. Gemini's startup includes a Java-like warm-up phase at ~1.4s.

## HTTP throughput

50 GET requests to `https://www.google.com/` with concurrency 5, routed through the MITM proxy.

| Metric | Value |
|--------|-------|
| Requests | 50/50 |
| Requests/sec | 58.3 |
| Transfer | 3.8MB |
| Total duration | 858ms |

| Latency percentile | Value |
|--------------------|-------|
| min | 56ms |
| p50 | 67ms |
| p95 | 250ms |
| p99 | 253ms |
| max | 254ms |

Latency includes the full path: guest -> net-proxy -> vsock -> host MITM proxy -> TLS termination -> internet -> re-encryption -> response. The p50 of 67ms reflects mostly internet RTT; the p95 tail is TLS session setup on new connections.

## Proxy throughput

100MB file download through the MITM proxy.

| Metric | Value |
|--------|-------|
| Downloaded | 100MB |
| Duration | 2.9s |
| Throughput | 34.3 MB/s |

This is the sustained bandwidth ceiling for the proxy pipeline (TLS termination + body inspection + re-encryption). Actual throughput varies with internet connection speed.

## Snapshot operations

End-to-end latency for snapshot operations via the MCP gateway at 3 workspace sizes. Each operation is a full round-trip: guest CLI -> vsock -> host gateway -> APFS filesystem -> response.

### 10 files

| Operation | Latency |
|-----------|---------|
| create | 879ms |
| list | 363ms |
| changes | 376ms |
| revert | 373ms |
| delete | 367ms |

### 100 files

| Operation | Latency |
|-----------|---------|
| create | 390ms |
| list | 367ms |
| changes | 377ms |
| revert | 377ms |
| delete | 400ms |

### 500 files

| Operation | Latency |
|-----------|---------|
| create | 394ms |
| list | 443ms |
| changes | 366ms |
| revert | 421ms |
| delete | 411ms |

The 10-file `create` is slower than 100/500 because it includes the first MCP handshake (JSON-RPC initialize). Subsequent operations reuse the connection. List and changes scale modestly with file count. The host gateway-side latency is typically 3-20ms -- the rest is vsock + MCP protocol overhead.

## Test environment

| Component | Version |
|-----------|---------|
| Host | macOS 15.4, Apple M4 Max |
| Capsem | v0.14.6 |
| Guest kernel | Linux 6.x (custom allnoconfig) |
| Storage | VirtioFS mode (APFS backing) |
| Python | 3.x (rootfs) |
| Node | v22.x (rootfs) |

## Reproducing

```bash
just bench    # Run all benchmarks (~2 min)
```

Results are displayed as rich tables in the terminal. JSON output is saved to `/tmp/capsem-benchmark.json` inside the VM.
