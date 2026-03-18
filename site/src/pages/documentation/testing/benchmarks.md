---
layout: ../../../layouts/Doc.astro
title: Benchmarks
description: VM performance benchmarks -- disk I/O, proxy throughput, CLI startup.
lastUpdated: "2026-03-11"
tags: ["testing", "performance", "benchmarks"]
---

Performance benchmarks for the Capsem VM sandbox. All benchmarks run inside the guest VM and output both a human-readable table (stderr) and machine-readable JSON (stdout).

## Running benchmarks

```bash
just bench                        # all benchmarks (boots VM once)
just run "capsem-bench disk"       # scratch disk I/O only
just run "capsem-bench rootfs"     # rootfs read only
just run "capsem-bench startup"    # CLI cold-start only
just run "capsem-bench http"       # HTTP latency only
just run "capsem-bench throughput" # proxy throughput only
```

`just bench` is part of `just full-test` and runs all modes in a single VM boot.

### Benchmark modes

| Mode | What it measures |
|------|-----------------|
| `disk` | Scratch disk sequential + random read/write (256 MB, ext4 on virtio-blk) |
| `rootfs` | Rootfs sequential + random read (squashfs via virtio-blk, read-only) |
| `startup` | CLI cold-start latency (drop_caches between runs, 3 runs each) |
| `http` | HTTP request latency (50 requests, 5 concurrent, through MITM proxy) |
| `throughput` | 100 MB download through the full MITM proxy pipeline |

## Current benchmarks (M4 Pro, 2026-03-11)

### Scratch disk I/O

Fresh ext4 volume on virtio-blk, formatted at every boot.

| Test | Throughput | IOPS | Duration |
|------|-----------|------|----------|
| Seq write (1MB blocks) | 1,018 MB/s | -- | 252 ms |
| Seq read (1MB blocks) | 5,369 MB/s | -- | 48 ms |
| Rand write (4K blocks) | 72 MB/s | 18,398 | 544 ms |
| Rand read (4K blocks) | 7,415 MB/s | 1,898,329 | 5 ms |

### Rootfs read I/O

Squashfs image mounted read-only via virtio-blk.

| Test | Detail | Throughput | IOPS | Duration |
|------|--------|-----------|------|----------|
| Seq read (1MB blocks) | codex binary (102 MB) | 748 MB/s | -- | 137 ms |
| Rand read (4K blocks) | 4,131 files sampled | 20 MB/s | 5,186 | 964 ms |

### CLI cold-start latency

3 runs per command, `drop_caches` before each.

| Command | Min | Mean | Max |
|---------|-----|------|-----|
| python3 | 8 ms | 9 ms | 10 ms |
| node | 135 ms | 137 ms | 138 ms |
| codex | 130 ms | 133 ms | 137 ms |
| claude | 336 ms | 342 ms | 346 ms |
| gemini | 1,012 ms | 1,066 ms | 1,121 ms |

### HTTP latency

50 requests to `google.com`, 5 concurrent, through the full MITM proxy.

| Metric | Value |
|--------|-------|
| Requests/sec | 15.2 |
| Latency min | 278 ms |
| Latency mean | 326 ms |
| Latency p50 | 292 ms |
| Latency p95 | 603 ms |
| Latency p99 | 624 ms |

### Proxy throughput

100 MB download through the complete data path: guest curl -> iptables REDIRECT -> capsem-net-proxy -> vsock -> host MITM proxy -> TLS termination + policy check -> upstream.

| Metric | Value |
|--------|-------|
| File size | 100 MB |
| Duration | 3.04s |
| Throughput | **32.9 MB/s** |
