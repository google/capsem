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
- [x] Benchmark async engine slice against current accepted stack.
- [x] Gate io_uring away from read-only rootfs and benchmark the gated slice.
- [x] Make io_uring opt-in by default and benchmark the safe default.
- [ ] Recover or explain scratch sequential read regression.
- [x] Add async-path telemetry counters for io_uring submissions/completions.
- [x] Implement and benchmark EROFS over virtio-pmem DAX as the final rootfs
      transport experiment before macOS reruns the shared rootfs candidates.
- [ ] Revisit Direct I/O in the EROFS+DAX plan: distinguish direct host-file
      pmem backing for DAX from virtio-blk `O_DIRECT` for fallback/rootfs-blk
      and scratch devices, then benchmark each independently.
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
- Active next slice: gate the Linux io_uring async block backend so scratch
  sequential-read gains do not regress rootfs and AI CLI startup.
- Current io_uring decision: keep the implementation and metrics, but default
  it off behind `CAPSEM_KVM_BLK_IO_URING` until a future tuning slice proves a
  clean default win.
- DAX direction: virtio-blk exposes `queue/dax=0`, so the meaningful EROFS DAX
  test is virtio-pmem. The first implementation maps a read-only copy of the
  rootfs image into guest physical memory, advertises it as virtio-pmem, and
  mounts `/dev/pmem0` with `-o dax` through a benchmark-only `erofs-dax` boot
  mode. This tests guest DAX and removes block I/O from rootfs reads; persistent
  host-file DAX can be evaluated later if the guest DAX signal is worth keeping.
- First DAX boot proof reached the guest and activated virtio device type 27,
  but mount failed with `erofs: dax options not supported`. Diagnosis: generic
  `CONFIG_FS_DAX` depends on `CONFIG_ZONE_DEVICE`, which in turn requires
  memory hotplug/hotremove plus sparse vmemmap. The defconfig now requests
  those dependencies explicitly before the rerun.
- Second DAX boot reached virtio-pmem but Linux rejected the namespace as
  misaligned. The KVM pmem mapping now aligns both guest physical start and
  advertised region size to 128 MiB so `ZONE_DEVICE` can map it.
- Enabling FS_DAX also pulled the virtio-fs guest driver into a DAX-sensitive
  path. Capsem's embedded virtio-fs does not expose a shared-memory DAX cache
  window, and we do not need virtio-fs DAX for rootfs-on-pmem, so the kernel
  defconfigs explicitly keep `CONFIG_FUSE_DAX` disabled.
- Direct I/O needs a revisit, but not as one global switch. The earlier
  rootfs virtio-blk `O_DIRECT` ablation was bad for compressed blk-backed
  rootfs. For EROFS+DAX, the more relevant question is direct host-file pmem
  backing: pad rootfs images to pmem alignment, map the file directly instead
  of copying into anonymous memory, and measure page faults, CPU time, and
  streaming throughput. Separately, rerun `O_DIRECT` for writable scratch and
  fallback rootfs-over-blk so non-DAX platforms still have a clean answer.

## Experiment Ledger

### Accepted: EROFS over virtio-pmem DAX experiment
- Code: this milestone commit.
- Bench: `benchmarks/kvm-rootfs-format-grid/data_1.2.1780320819_x86_64_1780357484.json`
- Archived superseded artifacts:
  `benchmarks/archive/benchmark-history-20260601T234525Z.zip`
- Proof:
  - `python3 -m py_compile scripts/kvm_rootfs_format_grid.py`
  - `uv run pytest tests/test_kvm_rootfs_format_grid.py -q`
  - `cargo test -p capsem-core pmem --lib`
  - `cargo test -p capsem-service process_env_allowlist_forwards_child_runtime_knobs`
  - `just build-kernel x86_64`
  - `just _pack-initrd`
  - `python3 scripts/kvm_rootfs_format_grid.py --formats erofs-lz4hc-c65536 --queue-counts 8 --queue-sizes 128 --seg-maxes 64 --logical-block-sizes 4096 --startup --pmem-dax --timeout 900`
- Mount proof:
  - `/run/capsem-lower`: `erofs` from `/dev/pmem0`
  - options: `ro,relatime,user_xattr,acl,cache_strategy=readaround,dax=always`
  - `/sys/block/pmem0/queue/dax`: `1`
- Result versus tuned EROFS virtio-blk baseline
  `data_1.2.1780320819_x86_64_1780351471.json`:
  - rootfs sequential read: 315.4 -> 276.8 MB/s (-12.2%)
  - rootfs random 4K read: 8626 -> 20875 IOPS (+142.0%)
  - large binary cold read: 578.0 -> 344.4 MB/s (-40.4%)
  - small JS reads: 237963 -> 546202 ops/s (+129.5%)
  - metadata stats: 35562 -> 123776 stats/s (+248.1%)
  - direct lower metadata stats: 37066 -> 172953 stats/s (+366.6%)
  - python startup min: 10.8 -> 6.4 ms (+40.7% faster)
  - node startup min: 51.2 -> 29.8 ms (+41.8% faster)
  - claude startup min: 502.5 -> 553.7 ms (-10.2% slower)
  - gemini startup min: 2070.1 -> 1958.0 ms (+5.4% faster)
  - codex startup min: 191.7 -> 137.9 ms (+28.1% faster)
- Decision: keep DAX as an opt-in experiment. It is clearly valuable for
  metadata/random/small-file lanes, but the large sequential regression means
  it is not a default rootfs transport candidate without more rootfs layout or
  read-ahead work.

### Accepted: EROFS DAX compressed versus uncompressed comparison
- Bench:
  `benchmarks/kvm-rootfs-format-grid/data_1.2.1780320819_x86_64_1780366089.json`
- Command:
  `python3 scripts/kvm_rootfs_format_grid.py --formats erofs-uncompressed,erofs-lz4hc-c65536 --queue-counts 8 --queue-sizes 128 --seg-maxes 64 --logical-block-sizes 4096 --startup --pmem-dax --timeout 900`
- Mount proof for both cells:
  - `/run/capsem-lower`: `erofs` from `/dev/pmem0`
  - options: `ro,relatime,user_xattr,acl,cache_strategy=readaround,dax=always`
  - `/sys/block/pmem0/queue/dax`: `1`
- Uncompressed EROFS DAX:
  - rootfs sequential read: 302.9 MB/s
  - rootfs random 4K read: 38881 IOPS
  - large binary cold read: 319.2 MB/s
  - small JS reads: 465352 ops/s
  - metadata stats: 114846 stats/s
  - direct lower metadata stats: 148592 stats/s
  - startup min: python 7.2 ms, node 18.3 ms, claude 344.8 ms,
    gemini 1859.4 ms, codex 87.4 ms
- Compressed `erofs-lz4hc-c65536` DAX, same run:
  - rootfs sequential read: 279.9 MB/s
  - rootfs random 4K read: 20042 IOPS
  - large binary cold read: 338.6 MB/s
  - small JS reads: 522448 ops/s
  - metadata stats: 123333 stats/s
  - direct lower metadata stats: 172544 stats/s
  - startup min: python 6.4 ms, node 29.7 ms, claude 605.8 ms,
    gemini 2061.7 ms, codex 134.0 ms
- Result, uncompressed versus compressed DAX:
  - rootfs sequential read: +8.2%
  - rootfs random 4K read: +94.0%
  - large binary cold read: -5.7%
  - small JS reads: -10.9%
  - metadata stats: -6.9%
  - direct lower metadata stats: -13.9%
  - python startup min: -12.5% slower
  - node startup min: +38.4% faster
  - claude startup min: +43.1% faster
  - gemini startup min: +9.8% faster
  - codex startup min: +34.8% faster
- Decision: uncompressed EROFS DAX is the best signal so far for random IOPS
  and AI CLI launch latency, but it is not an across-the-board winner. The next
  rootfs decision needs either repeated variance runs or a weighted product
  score that values startup/random I/O more than metadata-only probes.

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

### Measured Candidate: KVM virtio-blk io_uring async backend
- Code: `7037bac3 perf: add kvm virtio block io_uring backend`
- Bench: this milestone benchmark artifact commit.
- Proof:
  - `cargo test -p capsem-core hypervisor::kvm::virtio_blk::tests::block_async_notify_drains_from_eventfd_worker --lib`
  - `cargo test -p capsem-core hypervisor::kvm::virtio_blk::tests::block_io_uring_records_async_metrics --lib`
  - `cargo test -p capsem-core hypervisor::kvm::virtio_blk --lib`
  - `cargo test -p capsem-core hypervisor::kvm::virtio_queue --lib`
  - `cargo test -p capsem-core hypervisor::kvm::virtio_mmio --lib`
  - `cargo test -p capsem-core hypervisor::kvm --lib`
  - `just exec "echo ok"`
  - `just benchmark`
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
- Result versus previous Linux telemetry artifact:
  - disk sequential write: -0.6%
  - disk sequential read: +12.3%
  - disk random write IOPS: -0.1%
  - disk random read IOPS: -3.3%
  - rootfs sequential read: -16.3%
  - rootfs random 4K IOPS: -18.7%
  - large binary cold read: -18.7%
  - large binary warm read: -3.7%
  - small JS reads: -10.2%
  - metadata stats: -5.3%
  - python startup: +11.5% faster
  - node startup: -5.8% slower
  - claude startup: -6.8% slower
  - gemini startup: -6.6% slower
  - codex startup: -5.5% slower
- Interpretation:
  - As a default backend, this is not accepted yet. It improves scratch
    sequential reads and Python startup, but regresses rootfs reads, metadata,
    and AI CLI startup, which are higher-priority for the Linux landing.
  - Next experiment should keep the io_uring machinery but gate it, likely by
    device role or request shape: use io_uring where queue depth/sequential
    scratch I/O benefits, and keep the synchronous vectored path for rootfs or
    small/random read-heavy traffic unless further tuning reverses the loss.

### Measured Candidate: writable-device io_uring gate
- Code: `c2422adf perf: gate kvm io_uring block to writable disks`
- Bench: this milestone benchmark artifact commit.
- Hypothesis:
  - Rootfs is read-only and regressed badly under unconditional io_uring.
  - Scratch disk sequential read improved under io_uring.
  - Gating io_uring to writable block devices should recover rootfs/startup
    while preserving the scratch sequential-read gain.
- Proof target:
  - Unit test documents the gate: read-only devices stay on the synchronous
    vectored path, writable devices remain eligible for io_uring.
  - `cargo test -p capsem-core hypervisor::kvm --lib`
  - `just exec "echo ok"`
  - `just benchmark`
- Result versus previous Linux telemetry artifact:
  - disk sequential write: -3.0%
  - disk sequential read: -14.7%
  - disk random write IOPS: -0.4%
  - disk random read IOPS: -2.6%
  - rootfs sequential read: +4.9%
  - rootfs random 4K IOPS: -4.5%
  - large binary cold read: -0.1%
  - large binary warm read: +1.4%
  - small JS reads: -3.3%
  - metadata stats: +3.7%
  - python startup: +12.6% faster
  - node startup: +0.9% faster
  - claude startup: +0.1% faster
  - gemini startup: -1.8% slower
  - codex startup: -3.9% slower
- Recovery versus unconditional io_uring:
  - rootfs sequential read: +25.3%
  - rootfs random 4K IOPS: +17.5%
  - large binary cold read: +22.8%
  - small JS reads: +7.7%
  - metadata stats: +9.5%
  - node startup: +6.3% faster
  - claude startup: +6.4% faster
  - gemini startup: +4.5% faster
- Interpretation:
  - The gate fixed the worst rootfs/startup damage from unconditional io_uring,
    but it is still not a clean overall win against the telemetry baseline
    because disk sequential read regressed materially.
  - Next step should either disable io_uring by default while keeping the
    implementation for future tuning, or find a narrower request-shape gate
    that can recover disk sequential read without losing rootfs/startup.

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
  - io_uring candidate artifact is recorded separately and currently rejected
    as the default backend because rootfs/startup regressions outweigh scratch
    sequential read gains.
  - Writable-device io_uring gate recovered rootfs/startup versus unconditional
    io_uring but remains mixed versus telemetry because disk sequential read
    regressed.
  - Default-off io_uring is the current safe landing point: rootfs/random IOPS
    and most startup are neutral-to-better versus telemetry, while io_uring
    remains available for opt-in experiments.
- Missing/deferred:
  - macOS rerun for the event-index shared virtqueue/benchmark state.
  - clear explanation or recovery of scratch sequential read regression.

### Measured: default-off io_uring
- Code: `803bfbac perf: make kvm io_uring block opt in`
- Bench: this milestone benchmark artifact commit.
- Hypothesis:
  - The safest Linux landing default is the accepted synchronous vectored stack
    plus retained io_uring implementation behind `CAPSEM_KVM_BLK_IO_URING`.
  - This should recover the telemetry baseline while preserving the code path
    and metrics for future focused tuning.
- Proof target:
  - Gate unit test proves io_uring is default-off and opt-in for writable disks.
  - `cargo test -p capsem-core hypervisor::kvm --lib`
  - `just exec "echo ok"`
  - `just benchmark`
- Result versus previous Linux telemetry artifact:
  - disk sequential write: -0.8%
  - disk sequential read: -7.0%
  - disk random write IOPS: +1.6%
  - disk random read IOPS: +1.8%
  - rootfs sequential read: +3.0%
  - rootfs random 4K IOPS: +2.1%
  - large binary cold read: -4.1%
  - large binary warm read: -2.6%
  - small JS reads: -16.2%
  - metadata stats: +2.3%
  - python startup: +13.7% faster
  - node startup: +0.5% faster
  - claude startup: +0.0% faster
  - gemini startup: +1.0% faster
  - codex startup: -0.1% slower
- Interpretation:
  - This is the safest landing shape for the io_uring work: the implementation
    and telemetry are retained, but default execution returns to the accepted
    synchronous vectored path unless `CAPSEM_KVM_BLK_IO_URING` is set.
  - The remaining regressions in this run are not explained by default io_uring
    because the default path does not enable it. Disk sequential read and small
    JS reads should move to the next investigation loop with host-native
    variance and cache/rootfs attribution in view.

## Active Slice: disk sequential and small-JS attribution
- Build:
  - Attribute disk sequential read and small-JS regressions in the safe-default
    artifact without changing multiple knobs at once.
  - Use existing storage/rootfs benchmark lanes and host-native variance to
    decide whether this is backend behavior, rootfs cache/layout, or run noise.
- Do not build:
  - Additional io_uring tuning until the default-off artifact is committed and
    compared.
- Proof target:
  - `cargo test -p capsem-core hypervisor::kvm --lib`
  - `just exec "echo ok"`
  - `just benchmark`
