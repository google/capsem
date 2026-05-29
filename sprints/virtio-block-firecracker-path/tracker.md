# Sprint: Virtio Block Firecracker Path

## Tasks
- [x] Create sprint plan and tracker.
- [x] Record current combined KVM stack evidence.
- [ ] Add event-index feature negotiation and queue notification suppression.
- [ ] Benchmark event-index slice against `9d4c1f2a`.
- [ ] Prototype Linux async block engine with io_uring completion eventfd.
- [ ] Benchmark async engine slice against current accepted stack.
- [ ] Recover or explain scratch sequential read regression.
- [ ] Add hot-path telemetry counters for queue and backend behavior.
- [ ] Ask macOS team to rerun `just benchmark` for shared/rootfs-impacting changes.
- [ ] Commit accepted benchmark artifacts after each accepted milestone.
- [ ] Update `CHANGELOG.md` with each functional milestone.
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

## Coverage Ledger
- Unit/contract:
  - Current accepted stack passed focused KVM block, queue, and syscall tests.
- Functional:
  - Current accepted stack passed `just exec "echo ok"`.
- Adversarial:
  - Existing block/queue tests cover malformed descriptors, queue wrap, and
    worker quiesce. Event-index and async-error adversarial cases are pending.
- E2E/VM:
  - Current accepted stack passed canonical `just benchmark`.
- Telemetry:
  - Pending new counters for queue notify, suppression, sync/async operations,
    completions, and quiesce drain timing.
- Performance:
  - Current accepted benchmark artifact committed in `9d4c1f2a`.
- Missing/deferred:
  - macOS rerun for the combined stack.
  - event-index notification race tests.
  - io_uring async engine tests and VM proof.
  - clear explanation or recovery of scratch sequential read regression.
