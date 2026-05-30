# Sprint: Virtio Block Firecracker Path

## Tasks
- [x] Create sprint plan and tracker.
- [x] Record current combined KVM stack evidence.
- [x] Add event-index feature negotiation and queue notification suppression.
- [x] Benchmark event-index slice against `9d4c1f2a`.
- [x] Add OTel-ready virtio-blk queue/backend metrics and structured drain
      summaries.
- [x] Verify virtio-blk metrics with a local metrics recorder unit test.
- [ ] Prototype Linux async block engine with io_uring completion eventfd.
- [ ] Benchmark async engine slice against current accepted stack.
- [ ] Recover or explain scratch sequential read regression.
- [ ] Add async-path telemetry counters for io_uring submissions/completions.
- [ ] Ask macOS team to rerun `just benchmark` for shared/rootfs-impacting changes.
- [x] Commit accepted benchmark artifacts after each accepted milestone.
- [x] Update `CHANGELOG.md` with each functional milestone.
- [ ] Final gate and cleanup.

## Notes
- User pushed back correctly that isolated KVM experiments can hide compound
  effects. The sprint now treats Firecracker's path as a stack.
- Current accepted stack is `KVM_IOEVENTFD` plus used-ring batching. It improved
  Linux rootfs and most startup metrics, but regressed scratch sequential read.
- Firecracker's missing pieces in Capsem are event-index notification
  suppression and io_uring async completion depth.
- Cross-platform benefit is real for shared queue semantics, benchmark
  diagnostics, rootfs layout, and telemetry. Linux-only pieces must remain
  cleanly cfg-scoped so macOS can still run the same benchmark contract.
- Handoff rule from user: do the best Linux implementation, keep commits clean
  and documented, and let the macOS team pull the branch/main and validate with
  canonical `just benchmark`.
- Active next slice: add low-overhead virtio-blk metrics and structured queue
  summaries before the async engine, so the next benchmark explains queue depth,
  interrupt suppression, request mix, and backend drain time instead of only
  reporting end-to-end throughput.

## Experiment Ledger

### Accepted: combined KVM ioeventfd block batching
- Code: `ba8f260e perf: combine kvm ioeventfd block batching`
- Bench: `9d4c1f2a bench: record combined kvm block stack results`
- Proof:
  - `cargo test -p capsem-core hypervisor::kvm::virtio_blk --lib`
  - `cargo test -p capsem-core hypervisor::kvm::virtio_queue --lib`
  - `cargo test -p capsem-core hypervisor::kvm::sys --lib`
  - `just exec "echo ok"`
  - `just benchmark`
- Result versus previous Linux artifact:
  - rootfs sequential read: +8.5%
  - rootfs random 4K IOPS: +6.4%
  - rootfs metadata stats: +5.5%
  - disk random write IOPS: +3.6%
  - python startup: +23.4% faster
  - node startup: +1.1% faster
  - claude startup: +1.4% faster
  - gemini startup: +1.1% faster
  - disk sequential read: -13.1%
  - disk random read IOPS: -4.2%
  - large binary cold read: -4.7%
  - small JS reads: -2.9%
  - codex startup: -4.2%

### Accepted: KVM virtio-blk event-index notification suppression
- Code: this milestone commit.
- Bench: this milestone commit.
- Proof:
  - `cargo test -p capsem-core hypervisor::kvm::virtio_queue --lib`
  - `cargo test -p capsem-core hypervisor::kvm::virtio_blk --lib`
  - `cargo test -p capsem-core hypervisor::kvm::virtio_mmio --lib`
  - `cargo test -p capsem-core hypervisor::kvm --lib`
  - `just exec "echo ok"`
  - `just benchmark`
- Result versus `9d4c1f2a` Linux artifact:
  - disk sequential write: +3.6%
  - disk sequential read: +11.8%
  - disk random write IOPS: -0.1%
  - disk random read IOPS: +1.9%
  - rootfs sequential read: -10.4%
  - rootfs random 4K IOPS: -3.7%
  - large binary cold read: -0.6%
  - large binary warm read: -3.1%
  - small JS reads: +2.6%
  - metadata stats: +2.2%
  - python startup: +0.7% faster
  - node startup: -0.6% slower
  - claude startup: +0.1% faster
  - gemini startup: +0.9% faster
  - codex startup: -1.9% slower
- Follow-up:
  - Focused rootfs reruns with and without event-index advertised both landed
    around 141-142 MB/s sequential read, while the clean canonical artifact
    landed at 179.7 MB/s, so rootfs sequential read is still volatile and not
    explained by event-index negotiation alone.
  - Next slice should add queue/backend telemetry before more tuning so we can
    distinguish fewer interrupts from queue-depth, cache, and host I/O effects.

### Accepted: KVM virtio-blk telemetry counters
- Code: this milestone commit.
- Bench: pending clean-source benchmark artifact after this code commit.
- Proof:
  - `cargo test -p capsem-core hypervisor::kvm::virtio_blk::tests::block_read_records_queue_and_request_metrics --lib`
  - `cargo test -p capsem-core hypervisor::kvm::virtio_blk --lib`
  - `cargo test -p capsem-core hypervisor::kvm::virtio_queue --lib`
  - `cargo test -p capsem-core hypervisor::kvm::virtio_mmio --lib`
  - `cargo test -p capsem-core hypervisor::kvm --lib`
  - `just exec "echo ok"`
- Result:
  - No performance claim yet. This slice adds attribution needed before the
    async engine and rootfs recovery grind loop.

## Coverage Ledger
- Unit/contract:
  - Current accepted stack passed focused KVM block, queue, MMIO, and broader
    KVM library tests.
- Functional:
  - Current accepted stack passed `just exec "echo ok"`.
- Adversarial:
  - Existing block/queue tests cover malformed descriptors, queue wrap, worker
    quiesce, event-index kick suppression, and empty-queue notification arming.
    Async-error adversarial cases are pending.
- E2E/VM:
  - Current accepted stack passed canonical `just benchmark`.
- Telemetry:
  - KVM virtio-blk now emits metrics for notifications, queue drains,
    descriptors drained, used entries, request count/bytes/duration, interrupt
    raised/suppressed decisions, and quiesce drain timing.
- Performance:
  - Current accepted benchmark artifact included with the event-index slice.
- Missing/deferred:
  - macOS rerun for the event-index shared virtqueue/benchmark state.
  - io_uring async engine tests and VM proof.
  - clear explanation or recovery of scratch sequential read regression.

## Active Slice: virtio-blk telemetry
- Build:
  - `metrics` facade counters/histograms in the KVM virtio-blk path.
  - Structured drain logs with backend, notification count, descriptors,
    used entries, interrupt decision, and drain duration.
  - Quiesce drain duration metric for suspend/resume proof.
- Do not build:
  - New session DB tables in this slice. The source of truth is structured JSON
    logs plus the future OTel exporter; DB projection can be added when product
    workflows need persisted per-device rows.
- Proof target:
  - Unit test with `metrics_util::debugging::DebuggingRecorder` proving a read
    request emits request, byte, drain, used-entry, and interrupt metrics.
  - Focused KVM block tests plus `just exec "echo ok"`.
  - Broader `cargo test -p capsem-core hypervisor::kvm --lib` passed with
    317 tests after telemetry wiring.
