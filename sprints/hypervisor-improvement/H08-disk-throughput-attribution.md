# H08 - Disk Throughput Attribution

## Goal

Explain and improve the remaining Linux/KVM disk throughput gap from the code
path outward, not by isolated knob changes. This sprint treats rootfs DAX,
fallback virtio-blk rootfs, writable scratch, and network/RPS-facing I/O as
separate lanes so each result has a clear cause.

## Why This Exists

The current EROFS DAX candidate is a real improvement over the original Linux
rootfs: random IOPS, small-file reads, metadata stats, and AI CLI startup are
much better. The remaining weakness is raw sequential throughput and some
RPS-facing service paths. DAX rootfs reads no longer traverse the virtio-blk
worker, so more queue-size tuning cannot explain that lane. Writable scratch
and fallback block rootfs still do traverse virtio-blk and remain valid
hypervisor implementation targets.

## Scope

- Build a host-side and guest-side trace for each disk lane:
  - EROFS DAX rootfs over virtio-pmem;
  - EROFS/SquashFS rootfs over virtio-blk fallback;
  - writable scratch/system overlay over virtio-blk;
  - VirtioFS workspace where user-visible RPS or file-serving paths touch it.
- Add low-cardinality counters/timing that can flow through status and the
  existing OTel-ready metric-point contract:
  - request size distribution;
  - virtqueue batch depth;
  - queue-notify to drain latency;
  - drain to host syscall latency;
  - host syscall to completion latency;
  - interrupt decisions and used-ring publication batch size;
  - DAX pmem page-fault/cache evidence where available.
- Compare Capsem against Firecracker and crosvm by tracing the same lifecycle:
  guest request shape, virtqueue parsing, host memory translation, syscall,
  completion, used-ring update, and interrupt.
- Implement only changes that follow from the trace. Candidate areas:
  - virtio-blk request batching and completion publication;
  - direct I/O only for writable scratch and fallback rootfs-over-blk, not DAX;
  - io_uring engine shape for writable/fallback lanes;
  - host file cache policy and fd setup;
  - guest-visible block geometry if the trace proves request fragmentation.
- Keep the canonical benchmark contract intact. `just benchmark` remains the
  source of truth and must archive superseded artifacts.

## Out Of Scope

- New rootfs format decisions beyond proving how the transport affects
  throughput. EROFS DAX remains the current lead candidate until this sprint
  produces evidence to change it.
- Apple VZ implementation changes. Shared benchmark, rootfs, and telemetry
  changes must still be safe for the macOS team to run.
- CPU/SMP and memory optimization implementation. This sprint records the
  follow-up lanes, but disk attribution is the active code path.
- MITM/proxy redesign. RPS is included only where disk, workspace, or VM
  status polling clearly affects it.

## Work Plan

1. Baseline the accepted rootfs and scratch lanes against the active committed
   artifact, including host-native ratios and macOS comparison if available.
2. Add virtio-blk timing counters behind the existing metrics snapshot path.
3. Add DAX/rootfs cache evidence so DAX throughput can be separated from block
   backend throughput.
4. Run the trace benchmark and summarize request shapes before coding speedups.
5. Implement the first trace-backed virtio-blk improvement as one functional
   milestone.
6. Benchmark the change versus the immediately previous accepted artifact.
7. Repeat for at most two more trace-backed improvements before reassessing.

## Initial Baseline

Recorded on 2026-06-02 from committed benchmark artifacts before new H08 code:

- Canonical Linux artifact:
  `benchmarks/capsem-bench/data_1.2.1780320819_x86_64.json`, source commit
  `b834d5540a633c05616a3e2a1ce65f29e20aa5bf`, recorded dirty.
- Canonical macOS artifact:
  `benchmarks/capsem-bench/data_1.2.1780103109_arm64.json`.
- Active DAX rootfs candidate artifact:
  `benchmarks/kvm-rootfs-format-grid/data_1.2.1780320819_x86_64_1780402733.json`.

Canonical Linux versus macOS:

| Lane | Linux | macOS | Ratio |
| --- | ---: | ---: | ---: |
| Scratch seq read | 320.9 MB/s | 4043.0 MB/s | 0.08x |
| Scratch rand read | 7388 IOPS | 89809 IOPS | 0.08x |
| Rootfs seq read | 156.6 MB/s | 945.3 MB/s | 0.17x |
| Rootfs rand read | 2686 IOPS | 8734 IOPS | 0.31x |
| Rootfs large-binary cold read | 162.3 MB/s | 977.3 MB/s | 0.17x |
| Rootfs small-JS reads | 88791 ops/s | 399176 ops/s | 0.22x |
| Rootfs metadata stat | 58674 stats/s | 199915 stats/s | 0.29x |
| HTTP RPS | 54.8 rps | 65.7 rps | 0.83x |
| Proxy throughput | 17.43 Mb/s | 18.69 Mb/s | 0.93x |

Canonical Linux VM versus Linux host-native:

| Lane | VM | Host | Ratio |
| --- | ---: | ---: | ---: |
| Scratch seq read | 320.9 MB/s | 7048.0 MB/s | 0.046x |
| Scratch rand read | 7388 IOPS | 341675 IOPS | 0.022x |
| Scratch seq write | 155.1 MB/s | 441.2 MB/s | 0.35x |
| Scratch rand write | 2780 IOPS | 691 IOPS | 4.03x |

Active compressed EROFS DAX candidate, same Linux host:

| Lane | DAX candidate |
| --- | ---: |
| Rootfs seq read | 259.5 MB/s |
| Rootfs rand read | 22427 IOPS |
| Rootfs large-binary cold read | 332.6 MB/s |
| Rootfs small-JS reads | 564103 ops/s |
| Rootfs metadata stat | 121746 stats/s |
| Python startup mean | 11.0 ms |
| Node startup mean | 156.5 ms |
| Claude startup mean | 745.0 ms |
| Gemini startup mean | 2308.1 ms |
| Codex startup mean | 606.7 ms |

Interpretation:

- The DAX rootfs candidate already fixed much of the rootfs random/small-file
  problem versus the old canonical Linux artifact.
- The worst remaining disk gap is writable/fallback virtio-blk throughput:
  scratch read is only 0.08x macOS and 0.046x host-native.
- RPS is weaker than macOS, but not in the same class as disk throughput; keep
  RPS visible and revisit it after block/path attribution unless traces show a
  direct disk/workspace dependency.

## Acceptance Gates

- Every claimed improvement reports before/after percentages and identifies
  the lane: DAX rootfs, block rootfs, scratch/system overlay, VirtioFS, or RPS.
- New counters are visible through the same user-facing metrics path used by
  `capsem info`, service `/info`, gateway `/status`, and the OTel metric-point
  contract where applicable.
- At least one real VM run proves the counters move during disk activity.
- `just benchmark` records the accepted performance artifact, with host-native
  context and superseded-artifact archiving.
- Unit/contract tests cover counter math and request-shape classification.
- KVM-only code stays Linux-cfg isolated.

## 2026-06-02 Scratch Lane Correction

`capsem-bench disk` previously defaulted to `/root`, which is the host-visible
VirtioFS workspace in current storage mode, while the benchmark title called it
scratch disk I/O. The canonical disk lane now defaults to `/var/tmp` so the
disk benchmark measures writable scratch/system-overlay performance. `/root`
remains visible in `capsem-bench storage` for workspace/VirtioFS attribution.

Packaged VM measurements after changing the default, with an explicit `/root`
control run on the same code path:

| Lane | `/root` workspace | `/var/tmp` scratch/system | Improvement |
| --- | ---: | ---: | ---: |
| Sequential write | 121.3 MB/s | 174.1 MB/s | +43.5% |
| Sequential read | 522.4 MB/s | 809.1 MB/s | +54.9% |
| Random write | 615 IOPS | 2374 IOPS | +286.0% |
| Random read | 7903 IOPS | 563314 IOPS | +7028.9% |

This is a benchmark-contract/product-diagnostic improvement, not a claim that
VirtioFS `/root` got faster. The remaining hypervisor work is still raw
throughput attribution for writable/fallback virtio-blk and the workspace path.

## Current Open Work

- Prove the new KVM block request-shape counters move in a live VM during
  `capsem-bench disk` and `capsem-bench storage`.
- Record a fresh canonical `just benchmark` artifact now that the installed
  Linux build is working again.
- Add DAX/rootfs cache and page-fault evidence so DAX rootfs throughput is not
  confused with fallback virtio-blk throughput.
- Trace Capsem, Firecracker, and crosvm through the same block lifecycle:
  descriptor parsing, guest-memory translation, syscall, completion,
  used-ring publication, and interrupt.
- Land the first trace-backed KVM virtio-blk code-path speedup. The scratch
  lane correction fixed benchmark semantics and product diagnostics; it did
  not claim the underlying `/root` VirtioFS or raw virtio-blk path became
  faster.
- Revisit Direct I/O only for writable scratch and fallback rootfs-over-blk,
  separately from the EROFS DAX pmem path.
- Park CPU/SMP and memory follow-up slices with concrete metrics after disk
  attribution identifies which subsystem still dominates. RPS now has its own
  H09 attribution slice so it is not tracked only as a vague follow-up.

## 2026-06-02 Installed Build Handoff

Manual-test handoff is complete on Linux:

- Installed package version: `capsem 1.2.1780406785`.
- `capsem status --json`: state `ready`; service unit, assets, service
  endpoint, and gateway all `ok`; zero issues.
- Installed VM smoke: `capsem run "echo installed-capsem-ready-after-status-fix"`
  returned `installed-capsem-ready-after-status-fix`.
- Fixed status validation for Linux systemd units that reference
  `~/.capsem/bin/*` symlinks resolving to `/usr/bin/*`.

## Follow-Up Domains

- Memory: RSS attribution, guest working-set pressure, DAX page-fault behavior,
  and cache duplication between host and guest.
- CPU/SMP: vCPU exit reasons, host CPU burn per disk request, scheduler
  placement, and pause/resume lifecycle control.
- RPS: tracked explicitly in
  [H09 - Network And RPS Attribution](H09-network-rps-attribution.md).
