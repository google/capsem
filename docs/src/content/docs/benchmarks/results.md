---
title: Performance Results
description: Graph-first benchmark results for Capsem VM lifecycle, disk, app, and network performance.
sidebar:
  order: 1
---

| Artifact | Path |
|---|---|
| VM lifecycle | `benchmarks/lifecycle/data_1.3.1782571508.json` |
| Fork lifecycle | `benchmarks/fork/data_1.3.1782571508.json` |
| Guest benchmark | `benchmarks/capsem-bench/data_1.3.1782571508_arm64.json` |
| Route latency | `benchmarks/route-latency/data_1.3.1782571508.json` |

## VM lifecycle

```mermaid
xychart-beta
  title "Lifecycle mean latency (ms)"
  x-axis ["provision", "ready", "exec", "delete", "total"]
  y-axis "ms" 0 --> 1300
  bar [1132.2, 30.2, 28.5, 81.7, 1272.6]
```

```mermaid
xychart-beta
  title "Fork mean latency (ms)"
  x-axis ["fork", "boot_provision", "boot_ready"]
  y-axis "ms" 0 --> 1050
  bar [55.9, 1039.9, 37.0]
```

| Metric | Mean | p50 | p95 | Max |
|---|---:|---:|---:|---:|
| provision | 1132.2ms | 1094.9ms | 1196.1ms | 1207.3ms |
| exec_ready | 30.2ms | 30.3ms | 31.0ms | 31.1ms |
| exec | 28.5ms | 28.4ms | 28.8ms | 28.8ms |
| delete | 81.7ms | 78.2ms | 89.9ms | 91.2ms |
| total | 1272.6ms | 1232.4ms | 1344.7ms | 1357.2ms |
| fork | 55.9ms | - | - | 59.9ms |
| boot_provision | 1039.9ms | - | - | 1064.7ms |
| boot_ready | 37.0ms | - | - | 39.3ms |

## Disk

```mermaid
xychart-beta
  title "Rootfs format fresh run latency (s)"
  x-axis ["squashfs-zstd", "erofs-zstd15", "erofs-lz4hc12"]
  y-axis "s" 0 --> 10
  bar [9.10, 6.58, 6.05]
```

```mermaid
xychart-beta
  title "Rootfs format sequential read (MB/s)"
  x-axis ["squashfs-zstd", "erofs-zstd15", "erofs-lz4hc12"]
  y-axis "MB/s" 0 --> 4500
  bar [599.3, 1567.2, 4316.7]
```

```mermaid
xychart-beta
  title "Rootfs format random read (IOPS)"
  x-axis ["squashfs-zstd", "erofs-zstd15", "erofs-lz4hc12"]
  y-axis "IOPS" 0 --> 30000
  bar [7757, 19857, 28235]
```

```mermaid
xychart-beta
  title "Writable workspace throughput (MB/s)"
  x-axis ["seq_write", "seq_read", "rand_write", "rand_read"]
  y-axis "MB/s" 0 --> 3800
  bar [1792.8, 3715.8, 27.2, 171.6]
```

```mermaid
xychart-beta
  title "Writable workspace random IO (IOPS)"
  x-axis ["rand_write", "rand_read"]
  y-axis "IOPS" 0 --> 45000
  bar [6959.0, 43921.1]
```

| Metric | Value |
|---|---:|
| EROFS lz4hc-12 rootfs size | 720.5 MiB |
| EROFS lz4hc-12 fresh run | 6.05s |
| Rootfs largest binary sequential read | 2541.9 MB/s |
| Rootfs random 4K read | 29045.2 IOPS |
| Workspace sequential write | 1792.8 MB/s |
| Workspace sequential read | 3715.8 MB/s |
| Workspace random 4K write | 6959.0 IOPS |
| Workspace random 4K read | 43921.1 IOPS |

## App

```mermaid
xychart-beta
  title "CLI startup mean latency (ms)"
  x-axis ["python3", "node", "claude", "gemini", "codex"]
  y-axis "ms" 0 --> 820
  bar [3.8, 28.1, 137.0, 802.3, 116.9]
```

```mermaid
xychart-beta
  title "App file access throughput (MB/s)"
  x-axis ["large_cold", "large_warm", "small_js"]
  y-axis "MB/s" 0 --> 20000
  bar [2804.6, 19876.3, 5105.2]
```

```mermaid
xychart-beta
  title "App metadata scan (stats/s)"
  x-axis ["metadata_stat"]
  y-axis "stats/s" 0 --> 130000
  bar [125012.6]
```

| Metric | Min | Mean | Max |
|---|---:|---:|---:|
| python3 --version | 3.4ms | 3.8ms | 4.4ms |
| node --version | 26.9ms | 28.1ms | 29.0ms |
| claude --version | 134.8ms | 137.0ms | 138.2ms |
| gemini --version | 772.6ms | 802.3ms | 818.0ms |
| codex --version | 85.3ms | 116.9ms | 134.3ms |
| small JS reads | - | 572625.7 ops/s | - |
| metadata stat | - | 125012.6 stats/s | - |

## Network

```mermaid
xychart-beta
  title "VM-path requests per second"
  x-axis ["http_tiny", "model_json", "credential"]
  y-axis "RPS" 0 --> 3200
  bar [3098.3, 2477.2, 3092.8]
```

```mermaid
xychart-beta
  title "VM-path p50 latency (ms)"
  x-axis ["http_tiny", "model_json", "credential", "stats_read"]
  y-axis "ms" 0 --> 30
  bar [19.7, 25.1, 19.5, 0.352]
```

```mermaid
xychart-beta
  title "VM-path p95 latency (ms)"
  x-axis ["http_tiny", "model_json", "credential", "stats_read"]
  y-axis "ms" 0 --> 45
  bar [35.2, 40.7, 35.9, 0.449]
```

```mermaid
xychart-beta
  title "10 MiB transfer throughput (MB/s)"
  x-axis ["http_10mb"]
  y-axis "MB/s" 0 --> 70
  bar [64.7]
```

| Scenario | Success | RPS | Throughput | p50 | p95 | p99 |
|---|---:|---:|---:|---:|---:|---:|
| HTTP tiny | 50000/50000 | 3098.3 | 0.071 MB/s | 19.7ms | 35.2ms | 45.4ms |
| model_json_response | 50000/50000 | 2477.2 | 1.512 MB/s | 25.1ms | 40.7ms | 51.7ms |
| credential_response | 50000/50000 | 3092.8 | 0.652 MB/s | 19.5ms | 35.9ms | 45.5ms |
| service /stats read | 160/160 | - | - | 0.352ms | 0.449ms | 0.474ms |
| 10 MiB transfer | 1/1 | - | 64.7 MB/s | - | - | - |
