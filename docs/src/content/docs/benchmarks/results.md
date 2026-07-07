---
title: Performance Results
description: Current benchmark results for Capsem VM lifecycle, disk, app, network, and control-plane performance across macOS and Linux.
sidebar:
  order: 1
---

This page tracks the two release-relevant platforms. macOS arm64 remains the
performance baseline. Linux KVM x86_64 is now fully green in the release gate:
the numbers are acceptable for 1.5, with storage and cold lifecycle readiness
still the main gaps.

Linux release validation from the same run: main Python `1618 passed`, `78
skipped`, coverage `90.09%`; release-site shared dist `112 passed`; serial
timing and benchmark gates `17 passed`; build-chain/release serial suite `198
passed`, `2 skipped`, with the benchmark page contract fixed after the one
documentation failure; integration `95` guest diagnostics and `45` ledger
checks; cross-compiled Linux `.deb` `43 MB`; package boot `327` guest
diagnostics and `28` doctor checks; install E2E `81 passed`, `23 skipped`, `6
xfailed`.

### Compared Runs

- macOS arm64: `1.3.1782571508`
- Linux KVM x86_64: `1.4.1783187504`, July 7, 2026
- Linux artifacts: `benchmarks/capsem-bench/data_1.4.1783187504_x86_64.json`,
  `benchmarks/mock-server-protocol/data_1.4.1783187504_x86_64.json`,
  `benchmarks/lifecycle/data_1.4.1783187504.json`,
  `benchmarks/fork/data_1.4.1783187504.json`,
  `benchmarks/parallel/data_1.0.json`,
  `benchmarks/route-latency/data_1.4.1783187504.json`
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
  <text x="0" y="84" font-size="12" fill="currentColor">Linux KVM x86_64 1944.3ms</text>
  <rect x="160" y="71" width="400" height="18" fill="#0f766e" rx="3" />
</svg>

<svg viewBox="0 0 620 130" role="img" aria-label="VM fork benchmark" style="width:100%;max-width:760px;height:auto">
  <text x="0" y="18" font-size="14" font-weight="700" fill="currentColor">Fork mean, lower is better</text>
  <text x="0" y="48" font-size="12" fill="currentColor">macOS arm64 55.9ms, 15.1 MB image</text>
  <rect x="210" y="35" width="137" height="18" fill="#2563eb" rx="3" />
  <text x="0" y="84" font-size="12" fill="currentColor">Linux KVM x86_64 163.3ms, 17.5 MB image</text>
  <rect x="210" y="71" width="400" height="18" fill="#0f766e" rx="3" />
</svg>

Key numbers: Linux provision `792.7ms`, exec-ready `857.5ms`, running exec
`106.7ms`, delete `187.4ms`; macOS provision `1132.2ms`, exec-ready `30.2ms`,
running exec `28.5ms`, delete `81.7ms`. Linux 4-VM parallel benchmark completed
successfully in `132.18s`.

## Disk

Storage is the largest Linux performance gap. The 1.5 Linux image uses EROFS
with LZ4HC; the release result is correct and stable, but the KVM path is still
well behind macOS on scratch writes, rootfs reads, and metadata walking.

<svg viewBox="0 0 640 150" role="img" aria-label="Scratch sequential throughput benchmark" style="width:100%;max-width:780px;height:auto">
  <text x="0" y="18" font-size="14" font-weight="700" fill="currentColor">Scratch sequential throughput, higher is better</text>
  <text x="0" y="50" font-size="12" fill="currentColor">macOS write/read 1792.8 / 3715.8 MB/s</text>
  <rect x="240" y="37" width="174" height="16" fill="#2563eb" rx="3" />
  <rect x="420" y="37" width="180" height="16" fill="#60a5fa" rx="3" />
  <text x="0" y="92" font-size="12" fill="currentColor">Linux write/read 120.5 / 586.7 MB/s</text>
  <rect x="240" y="79" width="12" height="16" fill="#0f766e" rx="3" />
  <rect x="258" y="79" width="57" height="16" fill="#2dd4bf" rx="3" />
</svg>

<svg viewBox="0 0 640 150" role="img" aria-label="Root filesystem read benchmark" style="width:100%;max-width:780px;height:auto">
  <text x="0" y="18" font-size="14" font-weight="700" fill="currentColor">Rootfs and metadata reads, higher is better</text>
  <text x="0" y="50" font-size="12" fill="currentColor">macOS rootfs 2541.9 MB/s, metadata 125012.6 stats/s</text>
  <rect x="280" y="37" width="195" height="16" fill="#2563eb" rx="3" />
  <rect x="480" y="37" width="120" height="16" fill="#60a5fa" rx="3" />
  <text x="0" y="92" font-size="12" fill="currentColor">Linux rootfs 310.3 MB/s, metadata 32121.7 stats/s</text>
  <rect x="280" y="79" width="24" height="16" fill="#0f766e" rx="3" />
  <rect x="310" y="79" width="31" height="16" fill="#2dd4bf" rx="3" />
</svg>

Linux rootfs random 4K read is `7277.6 IOPS`; macOS is `29045.2 IOPS`. Linux
scratch random 4K write/read is `625.8 / 6904.0 IOPS`; macOS is `6959.0 /
43921.1 IOPS`. Linux large binary cold/warm reads are `377.9 / 5790.9 MB/s`;
macOS is `2804.6 / 19876.3 MB/s`.

## App

CLI startup on Linux is acceptable for the release, but consistently slower
than macOS. The slowest path is Gemini startup; AGY and other model client
ledger paths passed in Ironbank.

<svg viewBox="0 0 660 170" role="img" aria-label="CLI startup benchmark" style="width:100%;max-width:800px;height:auto">
  <text x="0" y="18" font-size="14" font-weight="700" fill="currentColor">CLI startup mean, lower is better</text>
  <text x="0" y="48" font-size="12" fill="currentColor">Python: macOS 3.8ms, Linux 18.9ms</text>
  <rect x="250" y="35" width="6" height="14" fill="#2563eb" rx="3" />
  <rect x="262" y="35" width="31" height="14" fill="#0f766e" rx="3" />
  <text x="0" y="78" font-size="12" fill="currentColor">Node: macOS 28.1ms, Linux 136.8ms</text>
  <rect x="250" y="65" width="23" height="14" fill="#2563eb" rx="3" />
  <rect x="278" y="65" width="113" height="14" fill="#0f766e" rx="3" />
  <text x="0" y="108" font-size="12" fill="currentColor">Claude: macOS 137.0ms, Linux 777.8ms</text>
  <rect x="250" y="95" width="23" height="14" fill="#2563eb" rx="3" />
  <rect x="278" y="95" width="127" height="14" fill="#0f766e" rx="3" />
  <text x="0" y="138" font-size="12" fill="currentColor">Gemini: macOS 802.3ms, Linux 2451.4ms</text>
  <rect x="250" y="125" width="131" height="14" fill="#2563eb" rx="3" />
  <rect x="386" y="125" width="200" height="14" fill="#0f766e" rx="3" />
</svg>

Additional app-side data: Linux Codex startup mean `398.7ms`; macOS Codex
startup mean `116.9ms`. Snapshot operations on Linux remain release-usable:
10-file create/list/changes/revert/delete is `2223.7 / 902.5 / 875.7 / 892.5 /
925.2ms`; 500-file path is `1114.7 / 920.8 / 979.7 / 886.5 / 904.8ms`.

## Network

Network protocol overhead is healthy enough for 1.5. Linux local HTTP is close
to macOS on small responses, Model protocol throughput is slightly higher, and
MCP plus DNS ledger checks passed through Ironbank. The large transfer path is
the main network gap.

<svg viewBox="0 0 660 160" role="img" aria-label="HTTP throughput benchmark" style="width:100%;max-width:800px;height:auto">
  <text x="0" y="18" font-size="14" font-weight="700" fill="currentColor">HTTP throughput, higher is better</text>
  <text x="0" y="50" font-size="12" fill="currentColor">Local HTTP /tiny: macOS 3098.3 rps, Linux 2617.6 rps</text>
  <rect x="330" y="37" width="240" height="16" fill="#2563eb" rx="3" />
  <rect x="330" y="65" width="203" height="16" fill="#0f766e" rx="3" />
  <text x="0" y="110" font-size="12" fill="currentColor">10 MiB HTTP transfer: macOS 64.7 MB/s, Linux 32.9 MB/s</text>
  <rect x="330" y="97" width="240" height="16" fill="#60a5fa" rx="3" />
  <rect x="330" y="125" width="122" height="16" fill="#2dd4bf" rx="3" />
</svg>

<svg viewBox="0 0 660 150" role="img" aria-label="Model and credential protocol benchmark" style="width:100%;max-width:800px;height:auto">
  <text x="0" y="18" font-size="14" font-weight="700" fill="currentColor">Model, credential, DNS, and MCP paths</text>
  <text x="0" y="50" font-size="12" fill="currentColor">Model protocol: macOS 2477.2 rps, Linux 2563.7 rps</text>
  <rect x="330" y="37" width="232" height="16" fill="#2563eb" rx="3" />
  <rect x="330" y="65" width="240" height="16" fill="#0f766e" rx="3" />
  <text x="0" y="110" font-size="12" fill="currentColor">Credential protocol: macOS 3092.8 rps, Linux 2288.5 rps</text>
  <rect x="330" y="97" width="240" height="16" fill="#60a5fa" rx="3" />
  <rect x="330" y="125" width="178" height="16" fill="#2dd4bf" rx="3" />
</svg>

Linux local HTTP p95 is `35.6ms`; macOS is `35.2ms`. Linux Model p95 is
`36.0ms`; macOS is `40.7ms`. Linux credential p95 is `41.3ms`; macOS is
`35.9ms`. Linux route contention remains comfortably under budget:
`/stats` p95 under profile writes is `1.336ms` against the `15ms` gate.
