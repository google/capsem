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

## Follow-Up Domains

- Memory: RSS attribution, guest working-set pressure, DAX page-fault behavior,
  and cache duplication between host and guest.
- CPU/SMP: vCPU exit reasons, host CPU burn per disk request, scheduler
  placement, and pause/resume lifecycle control.
- RPS: MITM/proxy request throughput, TUI/status polling overhead, and any
  disk/workspace dependency in serving paths.
