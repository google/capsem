---
title: Performance Results
description: Current benchmark results for Capsem VM lifecycle, disk, app, network, and control-plane performance across macOS and Linux.
sidebar:
  order: 1
---

This page tracks the two release-relevant platforms:

- **macOS arm64** is still the performance baseline.
- **Linux KVM x86_64** is now fully green in the release gate. The numbers are
  acceptable, but lifecycle readiness, EROFS reads, scratch I/O, CLI startup,
  and 10 MiB transfer throughput still trail macOS.

## Headline

<div style="display:grid;grid-template-columns:repeat(auto-fit,minmax(220px,1fr));gap:1rem;margin:1rem 0 1.5rem">
  <div style="border:1px solid var(--sl-color-gray-5);border-radius:8px;padding:1rem">
    <div style="font-size:0.85rem;color:var(--sl-color-gray-3)">macOS arm64 baseline</div>
    <div style="font-size:1.8rem;font-weight:700">1.27s</div>
    <div>Total VM lifecycle loop</div>
    <div style="font-size:0.85rem;color:var(--sl-color-gray-3)">1.3.1782571508</div>
  </div>
  <div style="border:1px solid var(--sl-color-gray-5);border-radius:8px;padding:1rem">
    <div style="font-size:0.85rem;color:var(--sl-color-gray-3)">Linux KVM x86_64</div>
    <div style="font-size:1.8rem;font-weight:700">1.94s</div>
    <div>Total VM lifecycle loop</div>
    <div style="font-size:0.85rem;color:var(--sl-color-gray-3)">1.4.1783187504</div>
  </div>
  <div style="border:1px solid var(--sl-color-gray-5);border-radius:8px;padding:1rem">
    <div style="font-size:0.85rem;color:var(--sl-color-gray-3)">Linux release gate</div>
    <div style="font-size:1.8rem;font-weight:700">pass</div>
    <div>Full `just test`, cross-compile, install E2E</div>
    <div style="font-size:0.85rem;color:var(--sl-color-gray-3)">July 7, 2026</div>
  </div>
</div>

## Readout

- **Linux is functionally ready.** The full Linux run passed unit, integration,
  Ironbank, benchmark, cross-compile, package boot, and install E2E gates.
- **Network protocol overhead is reasonable.** Linux guest model protocol is
  slightly above the macOS baseline (`2563.7` vs `2477.2` rps), while credential
  protocol is lower (`2288.5` vs `3092.8` rps).
- **Storage is the main Linux gap.** Scratch writes are `120.5 MB/s` on Linux
  versus `1792.8 MB/s` on macOS. Rootfs sequential reads are `310.3 MB/s`
  versus `2541.9 MB/s`.
- **Control-plane latency is still comfortably inside budget.** Linux `/stats`
  under profile writes has p95 `1.336ms` against a `15ms` gate.
- **Large transfer throughput is acceptable but lower.** Linux `/bytes/10mb`
  is `32.9 MB/s`; macOS is `64.7 MB/s`.

## Platform Comparison

Lower is better for latency. Higher is better for throughput and IOPS.

| Metric | macOS arm64 | Linux KVM x86_64 | Linux vs macOS |
|---|---:|---:|---:|
| Lifecycle loop mean | 1272.6ms | 1944.3ms | 1.53x slower |
| Exec-ready mean | 30.2ms | 857.5ms | 28.4x slower |
| Running exec mean | 28.5ms | 106.7ms | 3.74x slower |
| Fork mean | 55.9ms | 163.3ms | 2.92x slower |
| `/stats` contention p95 | 0.449ms | 1.336ms | 2.98x slower |

| Storage metric | macOS arm64 | Linux KVM x86_64 | Linux vs macOS |
|---|---:|---:|---:|
| Scratch sequential write | 1792.8 MB/s | 120.5 MB/s | 6.7% |
| Scratch sequential read | 3715.8 MB/s | 586.7 MB/s | 15.8% |
| Scratch random 4K write | 6959.0 IOPS | 625.8 IOPS | 9.0% |
| Scratch random 4K read | 43921.1 IOPS | 6904.0 IOPS | 15.7% |
| Rootfs sequential read | 2541.9 MB/s | 310.3 MB/s | 12.2% |
| Rootfs random 4K read | 29045.2 IOPS | 7277.6 IOPS | 25.1% |
| Large binary warm read | 19876.3 MB/s | 5790.9 MB/s | 29.1% |
| Metadata stat walk | 125012.6 stats/s | 32121.7 stats/s | 25.7% |

| Network metric | macOS arm64 | Linux KVM x86_64 | Linux vs macOS |
|---|---:|---:|---:|
| Local HTTP `/tiny` | 3098.3 rps | 2617.6 rps | 84.5% |
| Local HTTP p95 | 35.2ms | 35.6ms | roughly equal |
| Guest model protocol | 2477.2 rps | 2563.7 rps | 103.5% |
| Guest model p95 | 40.7ms | 36.0ms | faster |
| Guest credential protocol | 3092.8 rps | 2288.5 rps | 74.0% |
| Guest credential p95 | 35.9ms | 41.3ms | 1.15x slower |
| 10 MiB HTTP transfer | 64.7 MB/s | 32.9 MB/s | 50.9% |

## Linux Detail

Linux KVM x86_64 run: `1.4.1783187504`, July 7, 2026.

**Lifecycle:** provision `792.7ms`, exec-ready `857.5ms`, running exec
`106.7ms`, delete `187.4ms`, total loop `1944.3ms`. Fork mean is `163.3ms`;
forked image size is `17.5 MB`.

**EROFS/rootfs:** rootfs sequential read `310.3 MB/s`, random 4K read
`7277.6 IOPS`, large binary cold/warm reads `377.9 / 5790.9 MB/s`, small
JS/package reads `226137.5 ops/s`, metadata stat walk `32121.7 stats/s`.

**Scratch storage:** sequential write/read `120.5 / 586.7 MB/s`; random 4K
write/read `625.8 / 6904.0 IOPS`.

**CLI startup mean:** Python `18.9ms`, Node `136.8ms`, Claude `777.8ms`,
Gemini `2451.4ms`, Codex `398.7ms`.

**Network:** local HTTP `/tiny` `2617.6 rps` with p95 `35.6ms`; 10 MiB transfer
`32.9 MB/s`; guest model protocol `2563.7 rps`; guest credential protocol
`2288.5 rps`; all reported `0` failures.

**Snapshots:** 10-file create/list/changes/revert/delete:
`2223.7 / 902.5 / 875.7 / 892.5 / 925.2ms`. 500-file path:
`1114.7 / 920.8 / 979.7 / 886.5 / 904.8ms`.

**Parallel VM benchmark:** 4 VMs completed successfully in `132.18s`.

## macOS Detail

macOS arm64 baseline: `1.3.1782571508`.

**Lifecycle:** provision `1132.2ms`, exec-ready `30.2ms`, running exec
`28.5ms`, delete `81.7ms`, total loop `1272.6ms`. Fork mean is `55.9ms`;
forked image size is `15.1 MB`.

**Rootfs/storage:** rootfs sequential read `2541.9 MB/s`, random 4K read
`29045.2 IOPS`, large binary cold/warm reads `2804.6 / 19876.3 MB/s`, small
JS/package reads `572625.7 ops/s`, metadata stat walk `125012.6 stats/s`.
Scratch sequential write/read is `1792.8 / 3715.8 MB/s`; random 4K write/read
is `6959.0 / 43921.1 IOPS`.

**CLI startup mean:** Python `3.8ms`, Node `28.1ms`, Claude `137.0ms`,
Gemini `802.3ms`, Codex `116.9ms`.

**Network:** local HTTP `/tiny` `3098.3 rps` with p95 `35.2ms`; 10 MiB transfer
`64.7 MB/s`; guest model protocol `2477.2 rps`; guest credential protocol
`3092.8 rps`; all reported `0` failures.

## Release Gate Proof

Linux release validation from the same run:

- Main Python matrix: `1617 passed`, `78 skipped`, coverage `90.09%`.
- Release-site shared dist: `112 passed`.
- Serial timing and benchmark gates: `17 passed`.
- Build-chain and release serial suite: `199 passed`, `2 skipped`.
- Integration: `95` guest diagnostics and `45` ledger checks passed.
- Cross-compiled Linux `.deb`: `43 MB`, full `/usr/bin` binary cohort, package
  boot test passed `327` guest diagnostics and `28` doctor session checks.
- Install E2E: `81 passed`, `23 skipped`, `6 xfailed`.
- Workspace ownership after Docker cleanup: clean.

## Artifacts

| Platform | Artifact | Path |
|---|---|---|
| Linux x86_64 | Guest benchmark | `benchmarks/capsem-bench/data_1.4.1783187504_x86_64.json` |
| Linux x86_64 | Host protocol baseline | `benchmarks/mock-server-protocol/data_1.4.1783187504_x86_64.json` |
| Linux x86_64 | Lifecycle | `benchmarks/lifecycle/data_1.4.1783187504.json` |
| Linux x86_64 | Fork | `benchmarks/fork/data_1.4.1783187504.json` |
| Linux x86_64 | Parallel VMs | `benchmarks/parallel/data_1.0.json` |
| Linux x86_64 | Route latency | `benchmarks/route-latency/data_1.4.1783187504.json` |
| macOS arm64 | Guest benchmark | `benchmarks/capsem-bench/data_1.3.1782571508_arm64.json` |
| macOS arm64 | Lifecycle | `benchmarks/lifecycle/data_1.3.1782571508.json` |
| macOS arm64 | Fork | `benchmarks/fork/data_1.3.1782571508.json` |
| macOS arm64 | Route latency | `benchmarks/route-latency/data_1.3.1782571508.json` |
