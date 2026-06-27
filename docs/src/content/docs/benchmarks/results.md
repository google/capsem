---
title: Performance Results
description: Graph-first benchmark results for Capsem VM lifecycle, disk, app, and network performance.
sidebar:
  order: 1
---

| Artifact | Path |
|---|---|
| VM lifecycle | `benchmarks/lifecycle/data_1.3.1782571508.json` |
| Guest benchmark | `benchmarks/capsem-bench/data_1.3.1782571508_arm64.json` |
| DNS load | `benchmarks/release-hermetic/dns_load_blocked_c1_16_64_1.0.1780977620_arm64.json` |
| MCP load | `benchmarks/release-hermetic/mcp_load_c1_16_64_1.0.1780977620_arm64.json` |
| Route latency | `benchmarks/route-latency/data_1.3.1782571508.json` |

## VM lifecycle

<svg viewBox="0 0 720 300" role="img" aria-labelledby="vm-lifecycle-title" style="width:100%;height:auto;max-width:760px">
  <title id="vm-lifecycle-title">VM lifecycle mean latency in milliseconds</title>
  <rect x="0" y="0" width="720" height="300" fill="transparent" />
  <text x="20" y="30" font-size="20" font-weight="700" fill="currentColor">VM lifecycle mean latency (ms)</text>
  <line x1="70" y1="245" x2="680" y2="245" stroke="currentColor" stroke-opacity="0.35" />
  <line x1="70" y1="55" x2="70" y2="245" stroke="currentColor" stroke-opacity="0.35" />
  <rect x="125" y="79.7" width="72" height="165.3" fill="#2563eb" />
  <rect x="250" y="240.6" width="72" height="4.4" fill="#059669" />
  <rect x="375" y="240.8" width="72" height="4.2" fill="#d97706" />
  <rect x="500" y="233.1" width="72" height="11.9" fill="#7c3aed" />
  <text x="161" y="72" text-anchor="middle" font-size="14" fill="currentColor">1132.2</text>
  <text x="286" y="232" text-anchor="middle" font-size="14" fill="currentColor">30.2</text>
  <text x="411" y="232" text-anchor="middle" font-size="14" fill="currentColor">28.5</text>
  <text x="536" y="226" text-anchor="middle" font-size="14" fill="currentColor">81.7</text>
  <text x="161" y="270" text-anchor="middle" font-size="13" fill="currentColor">Provision</text>
  <text x="286" y="270" text-anchor="middle" font-size="13" fill="currentColor">Ready</text>
  <text x="411" y="270" text-anchor="middle" font-size="13" fill="currentColor">Exec</text>
  <text x="536" y="270" text-anchor="middle" font-size="13" fill="currentColor">Delete</text>
</svg>

| Metric | Mean | p50 | p95 | Max |
|---|---:|---:|---:|---:|
| Provision | 1132.2ms | 1094.9ms | 1196.1ms | 1207.3ms |
| Ready check | 30.2ms | 30.3ms | 31.0ms | 31.1ms |
| Exec | 28.5ms | 28.4ms | 28.8ms | 28.8ms |
| Delete | 81.7ms | 78.2ms | 89.9ms | 91.2ms |
| Total loop | 1272.6ms | 1232.4ms | 1344.7ms | 1357.2ms |

## Disk

<svg viewBox="0 0 720 300" role="img" aria-labelledby="disk-throughput-title" style="width:100%;height:auto;max-width:760px">
  <title id="disk-throughput-title">Disk sequential throughput in megabytes per second</title>
  <rect x="0" y="0" width="720" height="300" fill="transparent" />
  <text x="20" y="30" font-size="20" font-weight="700" fill="currentColor">Sequential throughput (MB/s)</text>
  <line x1="70" y1="245" x2="680" y2="245" stroke="currentColor" stroke-opacity="0.35" />
  <line x1="70" y1="55" x2="70" y2="245" stroke="currentColor" stroke-opacity="0.35" />
  <rect x="145" y="122.1" width="82" height="122.9" fill="#2563eb" />
  <rect x="315" y="158.3" width="82" height="86.7" fill="#059669" />
  <rect x="485" y="65.4" width="82" height="179.6" fill="#d97706" />
  <text x="186" y="114" text-anchor="middle" font-size="14" fill="currentColor">2541.9</text>
  <text x="356" y="150" text-anchor="middle" font-size="14" fill="currentColor">1792.8</text>
  <text x="526" y="58" text-anchor="middle" font-size="14" fill="currentColor">3715.8</text>
  <text x="186" y="270" text-anchor="middle" font-size="13" fill="currentColor">Rootfs read</text>
  <text x="356" y="270" text-anchor="middle" font-size="13" fill="currentColor">Workspace write</text>
  <text x="526" y="270" text-anchor="middle" font-size="13" fill="currentColor">Workspace read</text>
</svg>

<svg viewBox="0 0 720 300" role="img" aria-labelledby="disk-iops-title" style="width:100%;height:auto;max-width:760px">
  <title id="disk-iops-title">Disk random 4K input/output operations per second</title>
  <rect x="0" y="0" width="720" height="300" fill="transparent" />
  <text x="20" y="30" font-size="20" font-weight="700" fill="currentColor">Random 4K IOPS</text>
  <line x1="70" y1="245" x2="680" y2="245" stroke="currentColor" stroke-opacity="0.35" />
  <line x1="70" y1="55" x2="70" y2="245" stroke="currentColor" stroke-opacity="0.35" />
  <rect x="145" y="119.4" width="82" height="125.6" fill="#2563eb" />
  <rect x="315" y="214.9" width="82" height="30.1" fill="#059669" />
  <rect x="485" y="55" width="82" height="190" fill="#d97706" />
  <text x="186" y="111" text-anchor="middle" font-size="14" fill="currentColor">29045</text>
  <text x="356" y="207" text-anchor="middle" font-size="14" fill="currentColor">6959</text>
  <text x="526" y="48" text-anchor="middle" font-size="14" fill="currentColor">43921</text>
  <text x="186" y="270" text-anchor="middle" font-size="13" fill="currentColor">Rootfs read</text>
  <text x="356" y="270" text-anchor="middle" font-size="13" fill="currentColor">Workspace write</text>
  <text x="526" y="270" text-anchor="middle" font-size="13" fill="currentColor">Workspace read</text>
</svg>

| Metric | Value |
|---|---:|
| Rootfs sequential read | 2541.9 MB/s |
| Rootfs random 4K read | 29045.2 IOPS |
| Workspace sequential write | 1792.8 MB/s |
| Workspace sequential read | 3715.8 MB/s |
| Workspace random 4K write | 6959.0 IOPS |
| Workspace random 4K read | 43921.1 IOPS |
| 10 MiB HTTP transfer | 64.7 MB/s |

## App

<svg viewBox="0 0 720 300" role="img" aria-labelledby="app-startup-title" style="width:100%;height:auto;max-width:760px">
  <title id="app-startup-title">CLI startup mean latency in milliseconds</title>
  <rect x="0" y="0" width="720" height="300" fill="transparent" />
  <text x="20" y="30" font-size="20" font-weight="700" fill="currentColor">CLI startup mean latency (ms)</text>
  <line x1="70" y1="245" x2="680" y2="245" stroke="currentColor" stroke-opacity="0.35" />
  <line x1="70" y1="55" x2="70" y2="245" stroke="currentColor" stroke-opacity="0.35" />
  <rect x="105" y="244.1" width="66" height="0.9" fill="#2563eb" />
  <rect x="220" y="238.4" width="66" height="6.6" fill="#059669" />
  <rect x="335" y="212.5" width="66" height="32.5" fill="#d97706" />
  <rect x="450" y="55" width="66" height="190" fill="#7c3aed" />
  <rect x="565" y="217.3" width="66" height="27.7" fill="#dc2626" />
  <text x="138" y="236" text-anchor="middle" font-size="14" fill="currentColor">3.8</text>
  <text x="253" y="230" text-anchor="middle" font-size="14" fill="currentColor">28.1</text>
  <text x="368" y="205" text-anchor="middle" font-size="14" fill="currentColor">137.0</text>
  <text x="483" y="48" text-anchor="middle" font-size="14" fill="currentColor">802.3</text>
  <text x="598" y="210" text-anchor="middle" font-size="14" fill="currentColor">116.9</text>
  <text x="138" y="270" text-anchor="middle" font-size="13" fill="currentColor">python3</text>
  <text x="253" y="270" text-anchor="middle" font-size="13" fill="currentColor">node</text>
  <text x="368" y="270" text-anchor="middle" font-size="13" fill="currentColor">claude</text>
  <text x="483" y="270" text-anchor="middle" font-size="13" fill="currentColor">gemini</text>
  <text x="598" y="270" text-anchor="middle" font-size="13" fill="currentColor">codex</text>
</svg>

| Command | Min | Mean | Max |
|---|---:|---:|---:|
| python3 --version | 3.4ms | 3.8ms | 4.4ms |
| node --version | 26.9ms | 28.1ms | 29.0ms |
| claude --version | 134.8ms | 137.0ms | 138.2ms |
| gemini --version | 772.6ms | 802.3ms | 818.0ms |
| codex --version | 85.3ms | 116.9ms | 134.3ms |

## Network

<svg viewBox="0 0 720 300" role="img" aria-labelledby="network-rps-title" style="width:100%;height:auto;max-width:760px">
  <title id="network-rps-title">Network requests per second by workload</title>
  <rect x="0" y="0" width="720" height="300" fill="transparent" />
  <text x="20" y="30" font-size="20" font-weight="700" fill="currentColor">Requests per second</text>
  <line x1="70" y1="245" x2="680" y2="245" stroke="currentColor" stroke-opacity="0.35" />
  <line x1="70" y1="55" x2="70" y2="245" stroke="currentColor" stroke-opacity="0.35" />
  <rect x="115" y="142.1" width="78" height="102.9" fill="#2563eb" />
  <rect x="255" y="115.2" width="78" height="129.8" fill="#059669" />
  <rect x="395" y="55" width="78" height="190" fill="#d97706" />
  <rect x="535" y="162.7" width="78" height="82.3" fill="#7c3aed" />
  <text x="154" y="134" text-anchor="middle" font-size="14" fill="currentColor">3098</text>
  <text x="294" y="107" text-anchor="middle" font-size="14" fill="currentColor">3906</text>
  <text x="434" y="48" text-anchor="middle" font-size="14" fill="currentColor">5723</text>
  <text x="574" y="155" text-anchor="middle" font-size="14" fill="currentColor">2477</text>
  <text x="154" y="270" text-anchor="middle" font-size="13" fill="currentColor">HTTP</text>
  <text x="294" y="270" text-anchor="middle" font-size="13" fill="currentColor">DNS</text>
  <text x="434" y="270" text-anchor="middle" font-size="13" fill="currentColor">MCP</text>
  <text x="574" y="270" text-anchor="middle" font-size="13" fill="currentColor">Model</text>
</svg>

<svg viewBox="0 0 720 300" role="img" aria-labelledby="network-latency-title" style="width:100%;height:auto;max-width:760px">
  <title id="network-latency-title">Network p95 latency by workload in milliseconds</title>
  <rect x="0" y="0" width="720" height="300" fill="transparent" />
  <text x="20" y="30" font-size="20" font-weight="700" fill="currentColor">p95 latency (ms)</text>
  <line x1="70" y1="245" x2="680" y2="245" stroke="currentColor" stroke-opacity="0.35" />
  <line x1="70" y1="55" x2="70" y2="245" stroke="currentColor" stroke-opacity="0.35" />
  <rect x="115" y="80.7" width="78" height="164.3" fill="#2563eb" />
  <rect x="255" y="103.7" width="78" height="141.3" fill="#059669" />
  <rect x="395" y="141.0" width="78" height="104.0" fill="#d97706" />
  <rect x="535" y="55" width="78" height="190" fill="#7c3aed" />
  <text x="154" y="73" text-anchor="middle" font-size="14" fill="currentColor">35.2</text>
  <text x="294" y="96" text-anchor="middle" font-size="14" fill="currentColor">30.3</text>
  <text x="434" y="133" text-anchor="middle" font-size="14" fill="currentColor">22.3</text>
  <text x="574" y="48" text-anchor="middle" font-size="14" fill="currentColor">40.7</text>
  <text x="154" y="270" text-anchor="middle" font-size="13" fill="currentColor">HTTP</text>
  <text x="294" y="270" text-anchor="middle" font-size="13" fill="currentColor">DNS</text>
  <text x="434" y="270" text-anchor="middle" font-size="13" fill="currentColor">MCP</text>
  <text x="574" y="270" text-anchor="middle" font-size="13" fill="currentColor">Model</text>
</svg>

| Workload | RPS | p50 | p95 | p99 | Errors |
|---|---:|---:|---:|---:|---:|
| HTTP | 3098.3 | 19.7ms | 35.2ms | 45.4ms | 0 |
| DNS | 3905.5 | 14.3ms | 30.3ms | 34.9ms | 0 |
| MCP | 5723.4 | 9.0ms | 22.3ms | 27.0ms | 0 |
| Model | 2477.2 | 25.1ms | 40.7ms | 51.7ms | 0 |
