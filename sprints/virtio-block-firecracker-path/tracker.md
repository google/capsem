# Sprint: Virtio Block Firecracker Path

## Tasks
- [x] Create sprint plan and tracker.
- [x] Record current combined KVM stack evidence.
- [x] Add event-index feature negotiation and queue notification suppression.
- [x] Benchmark event-index slice against `9d4c1f2a`.
- [x] Add OTel-ready virtio-blk queue/backend metrics and structured drain
      summaries.
- [x] Verify virtio-blk metrics with a local metrics recorder unit test.
- [x] Benchmark virtio-blk telemetry slice against current accepted stack.
- [x] Prototype Linux async block engine with io_uring completion eventfd.
- [ ] Benchmark async engine slice against current accepted stack.
- [ ] Recover or explain scratch sequential read regression.
- [x] Add async-path telemetry counters for io_uring submissions/completions.
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
- Active next slice: benchmark the Linux io_uring async block backend against
  the current telemetry artifact, using queue/backend/request metrics to
  attribute any wins or regressions.

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
- Code: `4ca0fb0a feat: add kvm virtio block telemetry`
- Bench: this milestone benchmark artifact commit.
- Proof:
  - `cargo test -p capsem-core hypervisor::kvm::virtio_blk::tests::block_read_records_queue_and_request_metrics --lib`
  - `cargo test -p capsem-core hypervisor::kvm::virtio_blk --lib`
  - `cargo test -p capsem-core hypervisor::kvm::virtio_queue --lib`
  - `cargo test -p capsem-core hypervisor::kvm::virtio_mmio --lib`
  - `cargo test -p capsem-core hypervisor::kvm --lib`
  - `just exec "echo ok"`
  - `cargo test -p capsem-core hypervisor::kvm --lib`
  - `just exec "echo ok"`
  - `just benchmark`
- Result versus `3b2c7390` Linux event-index artifact:
  - disk sequential write: -3.9%
  - disk sequential read: -3.6%
  - disk random write IOPS: -3.3%
  - disk random read IOPS: -0.4%
  - rootfs sequential read: -3.8%
  - rootfs random 4K IOPS: -8.0%
  - large binary cold read: -6.1%
  - large binary warm read: -0.6%
  - small JS reads: -5.6%
  - metadata stats: -10.9%
  - python startup: -20.5% slower
  - node startup: -22.1% slower
  - claude startup: -14.6% slower
  - gemini startup: -5.5% slower
  - codex startup: -6.3% slower
- Interpretation:
  - This slice is accepted as an observability foundation, not a performance
    improvement. The recorded host-native artifact also moved during this run
    (for example native sequential read -18.6%), so the VM regressions need to
    be read with host-run variance in mind.
  - Keep the new metrics low overhead and use them to attribute the next async
    engine benchmark instead of tuning blind.

### Candidate: KVM virtio-blk io_uring async backend
- Code: this milestone commit.
- Bench: pending clean-source `just benchmark` artifact after the code commit.
- Proof so far:
  - `cargo test -p capsem-core hypervisor::kvm::virtio_blk::tests::block_async_notify_drains_from_eventfd_worker --lib`
  - `cargo test -p capsem-core hypervisor::kvm::virtio_blk::tests::block_io_uring_records_async_metrics --lib`
  - `cargo test -p capsem-core hypervisor::kvm::virtio_blk --lib`
  - `cargo test -p capsem-core hypervisor::kvm::virtio_queue --lib`
  - `cargo test -p capsem-core hypervisor::kvm::virtio_mmio --lib`
- Implementation notes:
  - The KVM ioeventfd worker now tries an io_uring backend first and falls back
    to the synchronous vectored worker when io_uring setup is unavailable.
  - Read/write requests submit GPA-translated scatter/gather iovecs directly to
    io_uring; completions publish used-ring entries and status bytes.
  - Completion eventfd plus epoll lets the worker react to both guest queue
    notifications and host I/O completions without blocking on individual
    preadv/pwritev calls.
  - Quiesce waits until in-flight async requests complete before replying, so
    checkpoint/suspend remains deterministic.
  - Metrics now cover async submissions, completions, fallback count, and
    in-flight depth.
- Result:
  - No performance claim until the clean-source benchmark artifact is recorded.

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
  - Current accepted stack passed canonical `just benchmark` after telemetry
    wiring.
- Telemetry:
  - KVM virtio-blk now emits metrics for notifications, queue drains,
    descriptors drained, used entries, request count/bytes/duration, interrupt
    raised/suppressed decisions, quiesce drain timing, io_uring submissions,
    completions, fallback count, and in-flight depth.
- Performance:
  - Current accepted benchmark artifact included with the telemetry slice.
- Missing/deferred:
  - macOS rerun for the event-index shared virtqueue/benchmark state.
  - io_uring VM proof and benchmark artifact.
  - clear explanation or recovery of scratch sequential read regression.

## Active Slice: io_uring benchmark
- Build:
  - Clean-source code commit for the io_uring backend.
  - Canonical `just benchmark` artifact recorded against that commit.
  - Sprint-local percent deltas versus `0bbd5397`.
- Do not build:
  - Additional tuning until this io_uring slice has its own benchmark. The user
    explicitly asked to benchmark each change separately.
- Proof target:
  - `cargo test -p capsem-core hypervisor::kvm --lib`
  - `just exec "echo ok"`
  - `just benchmark`
