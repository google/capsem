---
title: Performance Results
description: Current benchmark results for Capsem VM lifecycle, disk, app, network, and control-plane performance across macOS and Linux.
sidebar:
  order: 1
---

This page tracks the two release-relevant platforms. macOS arm64 remains the
performance baseline. Linux KVM x86_64 is fully exercising the 1.5 release gate:
the numbers are acceptable, with storage, snapshot latency, and cold lifecycle
readiness still the main Linux gaps.

Linux release validation from the current run: main Python `1622 passed`, `78
skipped`, coverage `90.09%`; release-site shared dist `112 passed`; serial
timing and benchmark gates `17 passed`; Rust line coverage `70.73%`. The
benchmark artifacts below are from the same Linux x86_64 EROFS + LZ4HC asset
set used by the release gate.

### Compared Runs

- macOS arm64: `1.3.1782571508`
- Linux KVM x86_64: `1.5.1783554373`, July 9, 2026
- Linux artifacts: `benchmarks/capsem-bench/data_1.5.1783554373_x86_64.json`,
  `benchmarks/mock-server-protocol/data_1.5.1783554373_x86_64.json`,
  `benchmarks/lifecycle/data_1.5.1783554373.json`,
  `benchmarks/fork/data_1.5.1783554373.json`,
  `benchmarks/parallel/data_1.0.json`,
  `benchmarks/route-latency/data_1.5.1783554373.json`
- macOS artifacts: `benchmarks/capsem-bench/data_1.3.1782571508_arm64.json`,
  `benchmarks/lifecycle/data_1.3.1782571508.json`,
  `benchmarks/fork/data_1.3.1782571508.json`,
  `benchmarks/route-latency/data_1.3.1782571508.json`

## VM lifecycle

Linux is slower than macOS, but remains well inside the release budgets. The
gap is concentrated in provision and exec-ready time; running exec latency and
forking are closer.

<svg viewBox="0 0 620 130" role="img" aria-label="VM lifecycle total loop benchmark" style="width:100%;max-width:760px;height:auto">
  <text x="0" y="18" font-size="14" font-weight="700" fill="currentColor">Total lifecycle loop, lower is better</text>
  <text x="0" y="48" font-size="12" fill="currentColor">macOS arm64 1272.6ms</text>
  <rect x="160" y="35" width="262" height="18" fill="#2563eb" rx="3" />
  <text x="0" y="84" font-size="12" fill="currentColor">Linux KVM x86_64 2006.5ms</text>
  <rect x="160" y="71" width="400" height="18" fill="#0f766e" rx="3" />
</svg>

<svg viewBox="0 0 620 130" role="img" aria-label="VM fork benchmark" style="width:100%;max-width:760px;height:auto">
  <text x="0" y="18" font-size="14" font-weight="700" fill="currentColor">Fork mean, lower is better</text>
  <text x="0" y="48" font-size="12" fill="currentColor">macOS arm64 55.9ms, 15.1 MB image</text>
  <rect x="210" y="35" width="132" height="18" fill="#2563eb" rx="3" />
  <text x="0" y="84" font-size="12" fill="currentColor">Linux KVM x86_64 169.3ms, 17.6 MB image</text>
  <rect x="210" y="71" width="400" height="18" fill="#0f766e" rx="3" />
</svg>

Key numbers: Linux provision `795.9ms`, exec-ready `921.9ms`, running exec
`98.5ms`, delete `190.2ms`; macOS provision `1132.2ms`, exec-ready `30.2ms`,
running exec `28.5ms`, delete `81.7ms`. Linux 4-VM parallel benchmark completed
successfully in `136.31s`.

## Disk

Storage is the largest Linux performance gap. The 1.5 Linux image uses EROFS
with LZ4HC; the release result is correct and stable, but the KVM path is still
well behind macOS on scratch writes, rootfs reads, and metadata walking.

<svg viewBox="0 0 640 150" role="img" aria-label="Scratch sequential throughput benchmark" style="width:100%;max-width:780px;height:auto">
  <text x="0" y="18" font-size="14" font-weight="700" fill="currentColor">Scratch sequential throughput, higher is better</text>
  <text x="0" y="50" font-size="12" fill="currentColor">macOS write/read 1792.8 / 3715.8 MB/s</text>
  <rect x="240" y="37" width="174" height="16" fill="#2563eb" rx="3" />
  <rect x="420" y="37" width="180" height="16" fill="#60a5fa" rx="3" />
  <text x="0" y="92" font-size="12" fill="currentColor">Linux write/read 116.5 / 389.6 MB/s</text>
  <rect x="240" y="79" width="12" height="16" fill="#0f766e" rx="3" />
  <rect x="258" y="79" width="38" height="16" fill="#2dd4bf" rx="3" />
</svg>

<svg viewBox="0 0 640 150" role="img" aria-label="Root filesystem read benchmark" style="width:100%;max-width:780px;height:auto">
  <text x="0" y="18" font-size="14" font-weight="700" fill="currentColor">Rootfs and metadata reads, higher is better</text>
  <text x="0" y="50" font-size="12" fill="currentColor">macOS rootfs 2541.9 MB/s, metadata 125012.6 stats/s</text>
  <rect x="280" y="37" width="195" height="16" fill="#2563eb" rx="3" />
  <rect x="480" y="37" width="120" height="16" fill="#60a5fa" rx="3" />
  <text x="0" y="92" font-size="12" fill="currentColor">Linux rootfs 296.2 MB/s, metadata 32919.8 stats/s</text>
  <rect x="280" y="79" width="23" height="16" fill="#0f766e" rx="3" />
  <rect x="310" y="79" width="32" height="16" fill="#2dd4bf" rx="3" />
</svg>

Linux rootfs random 4K read is `8520.2 IOPS`; macOS is `29045.2 IOPS`. Linux
scratch random 4K write/read is `581.8 / 6629.2 IOPS`; macOS is `6959.0 /
43921.1 IOPS`. Linux large binary cold/warm reads are `404.4 / 5573.0 MB/s`;
macOS is `2804.6 / 19876.3 MB/s`.

## App

CLI startup on Linux is acceptable for the release, but consistently slower
than macOS. The slowest path is Gemini startup; AGY and other model client
ledger paths passed in Ironbank.

<svg viewBox="0 0 660 170" role="img" aria-label="CLI startup benchmark" style="width:100%;max-width:800px;height:auto">
  <text x="0" y="18" font-size="14" font-weight="700" fill="currentColor">CLI startup mean, lower is better</text>
  <text x="0" y="48" font-size="12" fill="currentColor">Python: macOS 3.8ms, Linux 18.7ms</text>
  <rect x="250" y="35" width="6" height="14" fill="#2563eb" rx="3" />
  <rect x="262" y="35" width="31" height="14" fill="#0f766e" rx="3" />
  <text x="0" y="78" font-size="12" fill="currentColor">Node: macOS 28.1ms, Linux 139.0ms</text>
  <rect x="250" y="65" width="23" height="14" fill="#2563eb" rx="3" />
  <rect x="278" y="65" width="113" height="14" fill="#0f766e" rx="3" />
  <text x="0" y="108" font-size="12" fill="currentColor">Claude: macOS 137.0ms, Linux 885.9ms</text>
  <rect x="250" y="95" width="23" height="14" fill="#2563eb" rx="3" />
  <rect x="278" y="95" width="137" height="14" fill="#0f766e" rx="3" />
  <text x="0" y="138" font-size="12" fill="currentColor">Gemini: macOS 802.3ms, Linux 2586.6ms</text>
  <rect x="250" y="125" width="124" height="14" fill="#2563eb" rx="3" />
  <rect x="386" y="125" width="200" height="14" fill="#0f766e" rx="3" />
</svg>

Additional app-side data: Linux Codex startup mean `398.2ms`; macOS Codex
startup mean `116.9ms`. Snapshot operations on Linux remain release-usable:
10-file create/list/changes/revert/delete is `2455.4 / 974.6 / 1005.8 / 998.8 /
1010.6ms`; 500-file path is `1190.8 / 996.7 / 1081.9 / 1002.1 / 1008.2ms`.

## Network

Network protocol overhead is healthy enough for 1.5. Linux local HTTP is close
to macOS on small responses, model protocol throughput is slightly higher, and
MCP plus DNS ledger checks passed through Ironbank. The large transfer path is
the main network gap.

<svg viewBox="0 0 660 160" role="img" aria-label="HTTP throughput benchmark" style="width:100%;max-width:800px;height:auto">
  <text x="0" y="18" font-size="14" font-weight="700" fill="currentColor">HTTP throughput, higher is better</text>
  <text x="0" y="50" font-size="12" fill="currentColor">Local HTTP /tiny: macOS 3098.3 rps, Linux 2567.2 rps</text>
  <rect x="330" y="37" width="240" height="16" fill="#2563eb" rx="3" />
  <rect x="330" y="65" width="199" height="16" fill="#0f766e" rx="3" />
  <text x="0" y="110" font-size="12" fill="currentColor">10 MiB HTTP transfer: macOS 64.7 MB/s, Linux 32.3 MB/s</text>
  <rect x="330" y="97" width="240" height="16" fill="#60a5fa" rx="3" />
  <rect x="330" y="125" width="120" height="16" fill="#2dd4bf" rx="3" />
</svg>

<svg viewBox="0 0 660 150" role="img" aria-label="Model and credential protocol benchmark" style="width:100%;max-width:800px;height:auto">
  <text x="0" y="18" font-size="14" font-weight="700" fill="currentColor">Model, credential, DNS, and MCP paths</text>
  <text x="0" y="50" font-size="12" fill="currentColor">Model protocol: macOS 2477.2 rps, Linux 2508.4 rps</text>
  <rect x="330" y="37" width="237" height="16" fill="#2563eb" rx="3" />
  <rect x="330" y="65" width="240" height="16" fill="#0f766e" rx="3" />
  <text x="0" y="110" font-size="12" fill="currentColor">Credential protocol: macOS 3092.8 rps, Linux 2243.1 rps</text>
  <rect x="330" y="97" width="240" height="16" fill="#60a5fa" rx="3" />
  <rect x="330" y="125" width="174" height="16" fill="#2dd4bf" rx="3" />
</svg>

Linux local HTTP p95 is `36.0ms`; macOS is `35.2ms`. Linux model p95 is
`37.2ms`; macOS is `40.7ms`. Linux credential p95 is `42.0ms`; macOS is
`35.9ms`. Linux route contention remains comfortably under budget:
`/stats` p95 under profile writes is `1.511ms` against the `15ms` gate. The
Linux guest protocol lanes ran `50,000` requests per scenario with zero failed
requests; host-direct protocol lanes were `2461.9 rps` for model and `2217.1
rps` for credential.
