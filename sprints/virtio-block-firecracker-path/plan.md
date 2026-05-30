# Virtio Block Firecracker Path Sprint

## Goal
Move Capsem's block I/O architecture toward the Firecracker shape while keeping
the benchmark contract honest: one canonical `just benchmark` path, committed
artifacts, and side-by-side Linux/macOS evidence before declaring wins.

The immediate Linux stack (`KVM_IOEVENTFD` plus used-ring batching) improved
rootfs and most startup metrics but regressed scratch sequential reads. The next
step is not to back away from the shape; it is to add the missing parts that
make that shape pay off: event-index notification suppression, async I/O depth,
completion batching, and storage attribution.

## Current Evidence
- Accepted baseline before this sprint: vectored KVM block I/O via
  `preadv`/`pwritev` over GPA-translated guest memory.
- Combined Firecracker-shaped first stack:
  - `ba8f260e perf: combine kvm ioeventfd block batching`
  - `9d4c1f2a bench: record combined kvm block stack results`
- Combined stack result versus previous Linux artifact:
  - rootfs sequential read: +8.5%
  - rootfs random 4K IOPS: +6.4%
  - rootfs metadata stats: +5.5%
  - disk random write IOPS: +3.6%
  - python/node/claude/gemini startup: faster
  - disk sequential read: -13.1%
  - disk random read IOPS: -4.2%
  - large binary cold read: -4.7%
  - small JS reads: -2.9%
  - codex startup: -4.2%

## Firecracker Shape We Are Chasing
Firecracker's block path is a coordinated stack:
- `KVM_IOEVENTFD` maps guest queue notify writes to per-queue eventfds.
- An event manager owns queue, rate-limiter, activation, and async completion
  events.
- The block device drains with `pop_or_enable_notification()` so notification
  suppression and race handling are part of queue processing.
- `VIRTIO_RING_F_EVENT_IDX` is advertised and used to suppress redundant guest
  notifications and host interrupts.
- Used-ring entries are batched, then `used.idx` advances once.
- Async block mode submits raw guest-memory pointers to `io_uring`; completions
  arrive through a completion eventfd and are batched into the used ring.
- Fixed files and restricted io_uring operations keep the async path tight.

## Architecture Split

### Cross-platform / Apple-beneficial
- Virtio ring event-index feature negotiation and notification suppression.
- Shared queue semantics: deferred used entries, `used.idx` publishing, and
  interrupt suppression helpers.
- Request parsing, validation, and telemetry counters that can be shared by KVM
  and any future Apple-side virtio path or performance harness.
- Benchmark diagnostics for rootfs reads, metadata walks, startup, and host
  native attribution. macOS should rerun the same `just benchmark` artifact path.

### Linux-specific
- `KVM_IOEVENTFD` for virtio-mmio queue notify.
- Linux eventfd/epoll or event-manager worker plumbing.
- Optional `io_uring` async file engine.
- Linux block queue tuning and kernel parameter experiments.

### Apple validation lane
- Apple VZ may not expose our own block backend in the same way KVM does, so
  code changes must be clearly classified:
  - shared benchmark and virtio logic: macOS should run and compare;
  - Linux-only KVM/io_uring: macOS should compile or cfg-skip cleanly and still
    run canonical benchmarks;
  - rootfs/package format changes: macOS and Linux both rerun `just benchmark`.
- This sprint's Linux commits must be clean handoff points: documented,
  revertable, and paired with benchmark artifacts where performance is claimed.
  The macOS team should be able to pull `main`, run `just benchmark`, and commit
  only the resulting macOS artifacts without needing chat context.

## Planned Slices

### 1. Baseline and comparison guardrails
- Keep `9d4c1f2a` as the current measured Linux stack artifact.
- Add a sprint-local comparison table after every accepted benchmark.
- Keep generated artifacts committed only for accepted states.
- If an experiment is reverted, discard its artifacts and record the numbers in
  `tracker.md`.

### 2. Event-index notification suppression
- Add virtio feature bit support for `VIRTIO_RING_F_EVENT_IDX`.
- Extend `VirtQueue` with avail/used event helpers and `prepare_kick`-style
  interrupt decision logic.
- Use event-index semantics in the KVM block path first.
- Confirm macOS builds are unaffected and benchmark artifacts remain comparable.

### 3. Async block engine prototype
- Add a Linux-only async file engine abstraction beside the current sync
  vectored engine.
- Start with direct guest-memory pointers and io_uring single-buffer requests.
- Decide whether scatter/gather becomes multiple linked SQEs, `readv/writev`
  through io_uring, or temporary fallback to sync for multi-descriptor chains.
- Register completion eventfd and batch completions into the used ring.
- Preserve checkpoint quiesce by draining all pending async operations.

### 4. Queue/backend telemetry and attribution
- Add OTel-ready `metrics` counters and histograms for the current synchronous
  KVM virtio-blk path before introducing another backend:
  - queue notifications by backend (`mmio`, `ioeventfd`);
  - queue drains and descriptors drained per wake;
  - used entries published;
  - interrupts raised and suppressed;
  - request count/bytes/duration by operation and status;
  - queue drain duration and quiesce drain duration.
- Emit one structured `virtio.blk.queue_drain` summary per wake with enough
  fields to correlate benchmark artifacts with queue behavior, while keeping
  per-request logs at `trace`.
- Keep this slice free of session DB schema changes; the existing JSON process
  logs and future OTel exporter consume the same structured fields. Add a DB
  projection later only if product workflows need persisted VM-device rows.
- Use the telemetry slice as the baseline for the async engine so we can prove
  whether future gains come from deeper queueing, fewer interrupts, lower drain
  time, or faster host I/O.

### 5. Sequential read regression recovery
- Instrument whether the -13.1% scratch sequential regression is worker wake
  overhead, lost readahead, queue depth, host page cache behavior, or guest
  block queue parameters.
- Test queue-size/read-ahead/nr_requests changes through recorded benchmark
  artifacts.
- Keep rootfs/startup improvements while recovering scratch sequential read.

### 6. Storage/rootfs cross-platform tuning
- Use storage diagnostics to compare squashfs zstd block size, rootfs layout,
  overlay behavior, and metadata pressure across Linux and macOS.
- Only change rootfs chunk/compression/layout when both platforms can rerun the
  canonical benchmark.

### 7. Telemetry and observability
- Add long-term counters for queue notifications, descriptors drained, used
  entries published, interrupts raised/suppressed, sync vs async operations,
  io_uring submissions/completions, throttling, and quiesce drain duration.
- Make counters inspectable through logs/session artifacts without turning the
  hot path noisy.

### 8. Final proof and cleanup
- Commit accepted code and benchmark artifacts at each functional milestone.
- Run focused unit tests, `just exec "echo ok"`, and `just benchmark`.
- Ask macOS team to rerun `just benchmark` after any shared or rootfs-affecting
  milestone.
- For Linux-only KVM/io_uring milestones, macOS pickup still checks that the
  branch is cleanly pullable and that canonical benchmark artifacts remain
  comparable; performance judgment stays Linux-owned unless shared files changed.
- Update docs/skills if the canonical optimization workflow changes.

## Files Likely To Change
- `crates/capsem-core/src/hypervisor/kvm/virtio_queue.rs`
- `crates/capsem-core/src/hypervisor/kvm/virtio_blk.rs`
- `crates/capsem-core/src/hypervisor/kvm/virtio_mmio.rs`
- `crates/capsem-core/src/hypervisor/kvm/sys.rs`
- `crates/capsem-core/src/hypervisor/kvm/mod.rs`
- `guest/artifacts/capsem_bench/`
- `benchmarks/`
- `CHANGELOG.md`
- `sprints/virtio-block-firecracker-path/tracker.md`

## Done
- Linux block path has Firecracker-equivalent notification suppression,
  completion batching, and async I/O depth where it measurably helps.
- Rootfs/startup gains are preserved.
- Scratch sequential read and random read regressions are recovered or explained
  with evidence and accepted tradeoffs.
- macOS has rerun canonical artifacts for all shared/rootfs-impacting changes.
- Telemetry can explain real-world queue depth, latency, and backend behavior.
- Tracker and committed benchmark artifacts show every accepted and rejected
  experiment.

## Proof Matrix
- Unit/contract:
  - `cargo test -p capsem-core hypervisor::kvm::virtio_queue --lib`
  - `cargo test -p capsem-core hypervisor::kvm::virtio_blk --lib`
  - `cargo test -p capsem-core hypervisor::kvm::sys --lib`
- Functional:
  - `just exec "echo ok"`
  - focused `capsem-bench disk`, `rootfs`, and `storage` during grind loops
- Adversarial:
  - invalid descriptors, queue wrap, notification suppression races, async
    completion errors, quiesce with pending operations, unsupported KVM/io_uring
- E2E/VM:
  - canonical `just benchmark`
  - `capsem-doctor` after behavior that can affect boot/device correctness
- Telemetry:
  - inspect session/log counters for queue notify, completion, and suppression
    metrics after a real run
- Performance:
  - compare accepted benchmark artifacts against previous committed Linux and
    macOS artifacts with percentage deltas
- Missing/deferred:
  - Apple VZ internals are validated by the Apple team; this sprint owns keeping
    shared benchmark/rootfs changes compatible and comparable.
