# Sprint: hypervisor-improvement

## Tasks

- [x] Create meta-sprint structure and sub-sprint plan.
- [x] H00: close current KVM/block context and benchmark truth.
- [x] H00: make benchmark artifact retention part of `just benchmark`.
- [x] H01: safety and queue contracts.
  - [x] Record main merge and refreshed macOS benchmark comparison baseline.
  - [x] Add full guest-memory range validation before raw host pointers.
  - [x] Reject malformed virtqueue descriptor indices and cycles.
  - [x] Validate split-ring size, alignment, and guest-memory coverage.
  - [x] Reject invalid ready queues during virtio-mmio activation/restore.
  - [x] Make guest-memory offset arithmetic overflow-safe.
  - [x] Make virtio-blk aggregate descriptor length accounting overflow-safe.
- [ ] H03: observability, status, and OTel resource counters.
  - [x] Surface existing live VM resource metrics through service `/info`.
  - [x] Render live VM resource metrics in `capsem info`.
  - [x] Surface KVM virtio-blk queue/backend counters through metrics snapshots,
        service `/info`, and `capsem info`.
  - [x] Surface live resource and KVM block counters through gateway `/status`
        and the TUI session-info overlay.
  - [x] Add OTel-compatible metric-point mapping for live VM resource and KVM
        block counters.
  - [ ] Real OTLP exporter process/configuration remains deferred to the
        broader telemetry sprint.
- [ ] H02: event delivery and backpressure.
  - [x] Make KVM virtio-blk io_uring submission-queue saturation explicit
        backpressure instead of synchronous fallback.
  - [x] Add and surface `async_queue_full_total` through VM block metrics,
        service `/info`, `capsem info`, and the OTel metric-point contract.
  - [x] Retry backpressured KVM virtio-blk io_uring descriptors immediately
        after completions free submission capacity.
  - [x] Build the full Firecracker-shaped KVM block async profile before
        ablation: default io_uring engine for block devices, fixed registered
        fd, opcode probe, ring restrictions, explicit enable, existing
        backpressure, completion-triggered retry, and quiesce drain.
  - [x] Run the full-profile benchmark first, then grouped ablation.
  - [ ] Extend the same backpressure/event-loop audit to other KVM devices and
        completion paths after block is measured as a whole.
- [ ] H04: CPU, SMP, and lifecycle.
- [ ] H05: storage, rootfs, and filesystem experiments.
  - [x] Add a KVM block-shape profile covering queue count, queue size,
        segment limit, and logical block size.
  - [x] Add a focused gridsearch harness that records block-shape metadata and
        rootfs/startup results before choosing defaults.
  - [x] Add a rootfs-format grid harness so uncompressed rootfs and EROFS run
        through the same block-shape matrix as the current SquashFS baseline.
  - [ ] Test rootfs format/compression alternatives through the canonical
        benchmark path: uncompressed rootfs, EROFS, and a DAX-capability path
        if the current Linux/KVM transport can expose one cleanly.
- [ ] H06: benchmark and product proof.
  - [x] Add a crosvm reference harness for the same Capsem x86_64
        rootfs/startup workload used by the Firecracker comparison.
  - [x] Record crosvm epoll, corrected-uring, direct-I/O, and multi-worker
        lanes as structured benchmark artifacts.
  - [ ] Run a structured crosvm trace audit against Capsem instead of another
        keyword/source skim: follow the full path for block I/O, virtqueue
        descriptor handling, event-loop wakeups/batching, queue worker
        ownership, and rootfs/virtiofs-facing reads, then compare each path to
        the matching Capsem implementation and turn transferable differences
        into measured implementation slices.
- [ ] H07: docs, changelog, release gate.

## Notes

- User priority: improvements should include core systematic goodness, not only
  benchmark-visible wins.
- User priority: counters must become visible in status and available for
  OpenTelemetry.
- User priority: expose CPU usage, I/O, and memory usage so users get a clear
  system view.
- User priority: tune queue count, queue size, segment limit, and logical
  block size together; these are coupled, so isolated one-off constants are not
  enough.
- User hypothesis: rootfs format may be a first-order bottleneck. Test
  uncompressed rootfs to separate decompression CPU from host I/O, test EROFS
  as a read-only Linux-native alternative, and investigate DAX-style aggressive
  guest mapping for rootfs data before assuming more virtio-blk tuning is the
  main lever.
- Firecracker source audit found the strongest transferable patterns in vCPU
  control, event scheduling, virtqueue contracts, block engine configuration,
  io_uring restrictions/probes/backpressure, and hot-path metrics.
- Recommended first implementation after wrap-up: H01 range/queue safety, then
  H03 status/OTel counters.
- Benchmark retention is now policy: `just benchmark` archives superseded
  generated `data_*.json` artifacts after recording current artifacts.
- Linux x86_64 wrap-up benchmark rerun completed through canonical
  `just benchmark` on clean source commit `b6f9b6e2`; active artifacts were
  refreshed and the previous Linux artifacts were preserved in
  `benchmarks/archive/benchmark-prerun-20260530T123916Z.zip`.
- Current Linux/macOS comparison still shows Linux materially behind macOS:
  scratch read 0.11x, rootfs read 0.24x, startup python3 4.03x slower,
  startup node 2.68x slower, startup claude 4.13x slower, startup gemini
  4.21x slower, lifecycle total 2.44x slower, fork create 2.77x slower.
- `hypervisor-improvement` fast-forwarded to `origin/main` commit `238001fb`
  after the Linux support, TUI control, bug-fix, and refreshed macOS benchmark
  merges landed.
- Refreshed comparison after the macOS rerun now includes rootfs large-binary,
  small-JS, and metadata-stat lanes. Current Linux/macOS gap: scratch seq read
  0.10x, rootfs seq read 0.21x, rootfs metadata stat 0.21x, python startup
  3.83x slower, node startup 3.88x slower, lifecycle total 2.62x slower, fork
  create 3.16x slower.
- H01 is active first. Initial implementation slice: prove and fix complete
  `gpa + len` range validation before KVM virtio-blk zero-copy paths hand raw
  guest pointers to host syscalls.
- H01 first slice landed locally: `GuestMemoryRef::gpa_range_to_host` rejects
  overflow, RAM-end crossing, and x86_64 PCI-hole discontinuities; virtio-blk
  now uses it for zero-copy iovecs, discard reads, request header parsing,
  get-id writes, and status writes.
- H01 queue-contract slice landed locally: virtqueue pop now rejects invalid
  queue sizes, available-ring heads outside the queue, descriptor `next`
  indices outside the queue, and descriptor cycles instead of returning a
  partial or misparsed chain.
- H01 ring-layout slice landed locally: virtqueue operations now validate
  non-zero power-of-two size, descriptor-table 16-byte alignment, available-ring
  2-byte alignment, used-ring 4-byte alignment, and full guest-memory coverage
  for descriptor, available, and used rings before touching ring memory.
- H01 activation slice landed locally: virtio-mmio validates ready queue size,
  max-size, split-ring alignment, and full guest-memory coverage before
  `DRIVER_OK` activation or warm-restore reactivation. Invalid activation sets
  `STATUS_FAILED` and does not start device workers.
- H01 memory-helper slice landed locally: `GuestMemory` and `GuestMemoryRef`
  read/write helpers use checked offset arithmetic so invalid offsets produce
  errors rather than debug panics.
- H01 block-accounting slice landed locally: virtio-blk queue drains use
  checked `u32` accumulation for total descriptor data length so maliciously
  large chains return `IOERR` instead of panicking before I/O validation.
- H01 closed with `cargo test -p capsem-core hypervisor::kvm --lib` passing
  333 tests and `just exec "echo ok"` proving the current KVM boot/exec path
  still works after queue activation hardening. The old `just run` smoke path
  no longer exists after the TUI merge; `just exec` is the current one-shot VM
  command path.
- H03 is active next so the safety/queue counters and resource usage become
  visible through status and are ready for OTel export.
- H03 first slice landed locally: `/info` now projects the existing
  `VmMetricsSnapshot.resources` source of truth, and `capsem info` renders
  configured RAM/vCPUs, host PID/RSS/CPU time/CPU percent, and disk usage
  counters when they are available. Remaining H03 work is to wire queue/backend
  counters into status and the metrics/exporter surface.
- H03 second slice landed locally: KVM virtio-blk counters now accumulate in
  backend-owned atomics, remain emitted through the `metrics` facade, flow into
  `VmMetricsSnapshot.hypervisor.block`, and are projected through `/info` and
  `capsem info`. Live proof on a KVM VM reported 5,876 queue notifications,
  1,639 queue drains, 25,266 descriptors/used entries, 8,580 read ops, and
  31,394,816 block bytes read.
- H03 third slice landed locally: gateway `/status` enriches running VMs with
  `/info/{id}` metrics while keeping `/list` as the base/fallback, and the TUI
  session-info overlay renders resources, host RSS/CPU time, block ops, block
  bytes, and block queue counters. Live gateway proof reported 5,908 queue
  notifications, 1,638 queue drains, 25,264 descriptors/used entries, 8,578
  read ops, and 31,394,816 block bytes read for a throwaway KVM VM.
- H03 fourth slice landed locally: `VmMetricsSnapshot::otel_metric_points()`
  now flattens resource and KVM block counters into stable OTel-compatible
  metric points with explicit units, counter/gauge kinds, source metadata, and
  bounded attributes (`component`, `backend`). This makes the counters
  exporter-ready without adding a half-wired OTLP runtime in this sprint.
- H02 first slice landed locally: KVM virtio-blk io_uring submission queue
  saturation now backpressures instead of falling back to synchronous I/O. The
  worker records one queue-full event, rewinds the popped descriptor, leaves
  used/status untouched, and retries the same request when the async queue has
  capacity again.
- H02 second slice landed locally: the io_uring completion branch now reaps
  completions and immediately performs a completion-triggered queue drain. A
  descriptor rewound by SQ-full backpressure can be resubmitted as soon as
  completion capacity is available, without requiring a fresh guest notify.
- H02 direction correction on 2026-05-30: isolated VirtioFS batching/event-index
  experiments produced mixed numbers and were reverted uncommitted. The next
  accepted unit is the whole KVM block async profile, benchmarked as a complete
  backend shape before ablation. Firecracker comparison points being adopted
  now: async engine as a first-class file engine, fixed registered fd,
  restricted/probed ring, queue-full throttling/backpressure, completion event
  retry, deferred used-ring publication, event-index interrupt decisions, and
  quiesce drain semantics.
- H02 full-profile slice landed locally: KVM virtio-blk now uses the full async
  profile for read-only rootfs and writable block devices by default, keeps
  `CAPSEM_KVM_BLK_IO_URING=sync` as the ablation/fallback path, registers the
  backing fd as a fixed file, probes required opcodes, restricts the ring while
  disabled, explicitly enables it, and submits once per queue-drain or
  completion-retry batch.
- H02 full-profile benchmark, same-run async-vs-sync rootfs: seq read 121.0
  MB/s vs 121.7 (-0.6%), random read 1303 IOPS vs 1420 (-8.2%), large binary
  cold 170.9 MB/s vs 158.3 (+8.0%), large binary warm 5555.1 MB/s vs 5451.0
  (+1.9%), small JS 75,860 ops/s vs 73,875 (+2.7%), metadata stat 37,732/s vs
  36,196/s (+4.2%).
- H02 full-profile benchmark, same-run async-vs-sync startup: python3 38.3 ms
  vs 38.1 (-0.5%), node 336.7 ms vs 351.5 (+4.2%), claude 1720.9 ms vs
  1707.5 (-0.8%), gemini 3246.9 ms vs 3196.0 (-1.6%), codex 1203.5 ms vs
  1098.2 (-9.6%). Lower startup latency is better.
- H02 grouped ablation, io_uring depth 256 vs accepted 128: seq read 120.3
  MB/s vs 121.0 (-0.6%), random read 1347 IOPS vs 1303 (+3.4%), large binary
  cold 161.3 MB/s vs 170.9 (-5.6%), large binary warm 5555.1 MB/s vs 5555.1
  (+0.0%), small JS 71,505 ops/s vs 75,860 (-5.7%), metadata stat 39,430/s vs
  37,732/s (+4.5%). The mixed result rejected the larger ring for now.
- H02 VM smoke passed with the full async profile selected by default:
  `just exec "echo ok"` returned `ok` from a real KVM one-shot VM.
- Firecracker reality check on the same Linux host with official Firecracker
  v1.15.1, Capsem x86_64 rootfs.squashfs, Capsem kernel extracted from bzImage
  to ELF vmlinux, 2 vCPUs, 2048 MiB RAM, and a benchmark-only initrd: Firecracker
  Sync beat current Capsem full-async rootfs lanes by seq read +0.7%, random
  read +46.6%, cold large-binary +58.2%, warm large-binary +10.0%, small JS
  +21.7%, metadata stat +12.1%. Startup was also faster: python3 12.3%, node
  27.4%, claude 42.6%, gemini 23.3%, codex 36.4%.
- Firecracker Async was close to Sync for this workload, not a clean io_uring
  proof: vs current Capsem full-async it measured seq read +3.2%, random read
  +46.3%, cold large-binary +59.8%, warm large-binary +9.1%, small JS +27.4%,
  metadata stat +20.3%. This makes the next Capsem sprint less about blindly
  defaulting io_uring and more about matching Firecracker's virtqueue,
  interrupt, request, and guest-visible block behavior first.
- crosvm reference check, 2026-06-01: no packaged `crosvm` binary was available
  through apt, snap, or GitHub releases on this host, so the comparison uses a
  private source checkout built per crosvm's documented Linux path with a
  minimal no-default-features release build. This is reference evidence, not a
  Capsem product dependency.
- crosvm epoll with the same Capsem x86_64 kernel/rootfs/initrd shape beat
  Firecracker Sync on the rootfs lanes: seq read 123.3 MB/s (+1.1%), random
  read 2111 IOPS (+10.5%), cold large-binary 298.4 MB/s (+10.4%), small JS
  104,348 ops/s (+13.1%), metadata stat 48,030/s (+13.6%). Startup was similar
  or slightly better: python3 30.4 ms (+5.3%), node 243.5 ms (-0.5%), claude
  815.2 ms (+6.0%), gemini 2280.4 ms (+0.2%), codex 712.6 ms (+6.8%).
- crosvm epoll is still far from the committed macOS Capsem artifact: 0.13x seq
  rootfs read, 0.24x random IOPS, 0.31x cold large-binary read, 0.26x small JS,
  0.24x metadata stat, and roughly 2.8x-4.2x startup latency for the shared
  startup commands. That supports the hardware/host-storage hypothesis and the
  need to reason about overhead instead of treating any one Linux VMM as
  macOS-speed proof.
- crosvm `direct=true` is rejected for this read-mostly rootfs workload:
  seq read 63.7 MB/s, random 442 IOPS, cold large-binary 103.2 MB/s, small JS
  29,205 ops/s, metadata 14,580/s, and codex startup 1769.3 ms. Bypassing the
  host page cache made both cold and loader-style paths much worse.
- crosvm `multiple-workers=true` did not improve the default epoll shape:
  random read stayed similar at 2103 IOPS and cold large-binary stayed similar
  at 298.7 MB/s, but small JS dropped to 97,162 ops/s and metadata dropped to
  43,584/s. This argues against blindly adding more block workers without a
  measured queue/contention reason.
- crosvm `--async-executor uring` initially could not start because upstream
  crosvm's private `io_uring_setup` wrapper passed `io_uring_params` as an
  immutable reference even though the kernel writes ring offsets back into it.
  In the optimized release build, crosvm then computed a zero submit-ring mmap
  length and failed with `Failed to mmap submit ring ... Invalid argument`.
  A private reference patch changing that wrapper to `&mut io_uring_params`
  proved uring can boot on this host.
- crosvm uring after the private ABI fix is not faster than crosvm epoll on
  this read-heavy workload: seq read 121.7 MB/s (-1.3%), random read 2067 IOPS
  (-2.1%), cold large-binary 287.7 MB/s (-3.6%), small JS 103,633 ops/s
  (-0.7%), metadata 46,717/s (-2.7%), node startup 246.4 ms (-1.2%), claude
  867.4 ms (-6.4%), gemini 2332.6 ms (-2.3%), and codex 713.2 ms (-0.1%).
  The corrected lesson is that crosvm's cache-friendly epoll block path is the
  better reference here, not uring by itself.
- Next crosvm work must be a systematic trace comparison, not isolated knobs:
  trace request lifecycle from guest notification through descriptor parsing,
  backend scheduling, host I/O submission/completion, interrupt delivery, and
  rootfs-visible read behavior in crosvm, then map each step to Capsem's KVM
  path. The output should be a concrete delta table with expected benefit,
  complexity/maintenance cost, macOS/shared applicability, and the benchmark
  lane that would prove or reject it.
- crosvm/Firecracker source audit, first accepted Capsem slice: crosvm
  advertises `VIRTIO_BLK_F_SEG_MAX` and `VIRTIO_BLK_F_BLK_SIZE`, with
  `seg_max` bounded by the queue size, while Firecracker keeps a simple
  single-queue device shape. Capsem now reports `seg_max = queue_size - 2` and
  `blk_size = 512` before attempting higher-risk multi-queue work, so Linux can
  use explicit block geometry without changing the async backend contract.
- Focused live KVM check for that slice confirmed Linux sees
  `/sys/block/vda/queue/max_segments = 254` and `logical_block_size = 512`.
  Against the committed Linux baseline artifact, the same live `capsem-bench
  rootfs` probe measured random read 1,463 IOPS (+13.9%), cold large-binary
  181.4 MB/s (+12.3%), small JS 78,261 ops/s (+4.6%), metadata 39,394 stats/s
  (+10.4%), warm large-binary 5,468.8 MB/s (-1.6%), and sequential read
  129.2 MB/s (-23.6%). This is a focused experiment, not a replacement for a
  canonical `just benchmark` artifact.
- H05 first block-shape slice landed locally: KVM virtio-blk now accepts
  bounded `CAPSEM_KVM_BLK_QUEUE_COUNT`, `CAPSEM_KVM_BLK_QUEUE_SIZE`,
  `CAPSEM_KVM_BLK_SEG_MAX`, and `CAPSEM_KVM_BLK_LOGICAL_BLOCK_SIZE` knobs,
  advertises `VIRTIO_BLK_F_MQ` plus config `num_queues` when queue count is
  greater than one, and registers one x86_64 `KVM_IOEVENTFD` datamatch per
  queue so MQ benchmarks do not fall back to vCPU MMIO exits. `capsem-service`
  now forwards those numeric tuning knobs to `capsem-process`.
- Focused live KVM MQ probe with `queue_count=4`, `queue_size=128`,
  `seg_max=64`, and `logical_block_size=4096` confirmed Linux sees
  `/sys/block/vda/mq` with 4 queues, `max_segments=64`,
  `logical_block_size=4096`, and `nr_requests=64`. Against the committed Linux
  baseline artifact, the same live `capsem-bench rootfs` probe measured random
  read 3,022 IOPS (+135.2%), cold large-binary 179.2 MB/s (+11.0%), small JS
  106,595 ops/s (+42.5%), metadata 64,006 stats/s (+79.4%), warm large-binary
  5,354.9 MB/s (-3.7%), and sequential read 134.0 MB/s (-20.8%). This is a
  focused experiment and will feed the gridsearch rather than being accepted as
  the default.
- H05 gridsearch harness landed locally as `scripts/kvm_block_shape_grid.py`.
  It expands queue count, queue size, segment limit, and logical block size as
  a coupled matrix, runs the selected shapes through `just exec`, captures
  Linux sysfs queue state, and writes structured artifacts under
  `benchmarks/kvm-block-shape/`. A one-cell harness proof for
  `queue_count=4`, `queue_size=128`, `seg_max=64`, `logical_block_size=4096`
  wrote `benchmarks/kvm-block-shape/data_1.2.1780320819_x86_64_1780334268.json`
  with sysfs `mq_dirs=4`, `max_segments=64`, `logical_block_size=4096`,
  `nr_requests=64`, and rootfs random read 2,885 IOPS, small JS 109,911 ops/s,
  metadata 61,877 stats/s.
- H05 first real grid recorded
  `benchmarks/kvm-block-shape/data_1.2.1780320819_x86_64_1780334747.json`
  for 24 queue/geometry cells: queue counts 1/4/8, queue sizes 128/256,
  segment max auto/64, logical block sizes 512/4096. The best balanced cell by
  equal rootfs lane ratio was `queue_count=8`, `queue_size=128`, `seg_max=64`,
  `logical_block_size=4096`: random read 3,349 IOPS (+160.7% vs committed
  Linux baseline), small JS 116,278 ops/s (+55.4%), metadata 63,880 stats/s
  (+79.1%), cold large-binary 203.2 MB/s (+25.8%), sequential read 138.1 MB/s
  (-18.4%). Linux exposed 4 `/sys/block/vda/mq` queues for both 4-queue and
  8-queue device configs on this 2-vCPU VM, so the next grid should include
  vCPU count and avoid assuming requested queue count equals active Linux
  hardware queues.
- H05 candidate startup proof recorded
  `benchmarks/kvm-block-shape/data_1.2.1780320819_x86_64_1780334834.json`
  for `queue_count=8`, `queue_size=128`, `seg_max=64`,
  `logical_block_size=4096` with startup enabled. Rootfs stayed in the same
  improved band: random read 3,502 IOPS (+172.6%), small JS 105,614 ops/s
  (+41.1%), metadata 61,219 stats/s (+71.6%), cold large-binary 199.3 MB/s
  (+23.4%), sequential read 144.2 MB/s (-14.8%). Startup also improved versus
  the committed Linux baseline: python3 31.1 ms (+30.2% faster), node
  247.3 ms (+44.8%), claude 1,301.8 ms (+30.8%), gemini 2,950.2 ms (+10.2%),
  codex 820.5 ms (+36.0%). This candidate is strong enough for a default
  experiment, but still needs canonical `just benchmark` and macOS/shared
  impact review before acceptance.
- H05 scope split landed locally after writable disk checks showed the rootfs
  candidate must not be applied globally. `CAPSEM_KVM_BLK_ROOTFS_*` overrides
  now target read-only rootfs devices, `CAPSEM_KVM_BLK_WRITABLE_*` can target
  writable block devices, and generic `CAPSEM_KVM_BLK_*` remains a fallback.
  Live sysfs proof with only rootfs-specific knobs showed `vda` at 4 active MQ
  queues, `max_segments=64`, `logical_block_size=4096`, `nr_requests=64`, while
  `vdb` stayed at the default 1 queue, `max_segments=254`,
  `logical_block_size=512`, `nr_requests=128`. Focused disk probes still showed
  low current write IOPS even with default writable geometry, so accepting the
  rootfs candidate requires canonical `just benchmark` rather than isolated
  disk interpretation.
- H05 canonical rootfs-only benchmark ran through `just benchmark` with
  `CAPSEM_KVM_BLK_ROOTFS_QUEUE_COUNT=8`, `CAPSEM_KVM_BLK_ROOTFS_QUEUE_SIZE=128`,
  `CAPSEM_KVM_BLK_ROOTFS_SEG_MAX=64`, and
  `CAPSEM_KVM_BLK_ROOTFS_LOGICAL_BLOCK_SIZE=4096` on source commit `b834d554`.
  The run refreshed the active Linux x86_64 artifacts and preserved the prior
  active generated artifacts in local archive
  `benchmarks/archive/benchmark-prerun-20260601T180613Z.zip`. Artifact metadata
  recorded source-clean git state (`source_dirty=false`) plus generated
  security-engine artifacts already modified by the same benchmark run. The
  canonical suite generated all requested performance artifacts but failed the
  endpoint-latency gate: service global endpoints were roughly 3.3-6.1 ms p95
  against the 3 ms gate, and `/logs/{id}` was roughly 25.3-26.3 ms p95 against
  the 12 ms gate.
- H05 next experiment candidates from user review: rootfs format/compression
  matrix should include current SquashFS zstd baseline, uncompressed SquashFS
  or equivalent uncompressed read-only image, EROFS if the guest kernel has or
  can reasonably gain support, and DAX-style guest mapping if the KVM/rootfs
  transport can expose it without replacing the product architecture. The
  benchmark proof must stay canonical: `capsem-bench storage/rootfs/startup`
  plus host-native/raw-system comparison and Linux/macOS artifact comparison.
  DAX is explicitly a capability audit first, because virtiofs DAX, pmem DAX,
  and block-backed read-only rootfs have different kernel/device requirements.
- H05 rootfs-format grid harness landed locally: `scripts/kvm_rootfs_format_grid.py`
  materializes benchmark-only asset roots under `target/kvm-rootfs-format-grid`,
  currently covering current SquashFS zstd, SquashFS with compression disabled,
  and EROFS image generation. Each format runs through the same
  queue-count/queue-size/segment/logical-block-size shape matrix via `just exec`
  and records guest sysfs queue state plus `capsem-bench storage`, `rootfs`, and
  optional `startup` JSON. The harness records DAX as `not_implemented` rather
  than pretending the current virtio-blk rootfs path can exercise DAX; real DAX
  needs a separate virtiofs-DAX or pmem-style mapping path.
- H05 first rootfs-format grid artifact:
  `benchmarks/kvm-rootfs-format-grid/data_1.2.1780320819_x86_64_1780339109.json`
  compared current SquashFS zstd (496.4 MiB) against uncompressed SquashFS
  (1,603.5 MiB) across the same two rootfs-only block shapes:
  `queue_count=1/8`, `queue_size=128`, `seg_max=64`,
  `logical_block_size=4096`, with `storage`, `rootfs`, and `startup` enabled.
  Uncompressed SquashFS won most read/startup lanes despite the 3.2x larger
  image. Best uncompressed vs best zstd: rootfs seq 195.4 vs 147.7 MB/s
  (+32.3%), random read 3,419 vs 3,184 IOPS (+7.4%), cold large-binary 313.3
  vs 191.6 MB/s (+63.5%), small-JS 127,580 vs 99,656 ops/s (+28.0%), python
  startup 20.5 vs 29.5 ms (+30.5% faster), node 143.5 vs 266.1 ms
  (+46.1%), claude 1,044.4 vs 1,321.6 ms (+21.0%), gemini 2,728.9 vs
  2,970.1 ms (+8.1%), and codex 492.4 vs 872.7 ms (+43.6%). Metadata went the
  other way in this small grid: best zstd 62,234 stats/s vs best uncompressed
  53,527 stats/s (-14.0%). This is strong evidence that rootfs compression is
  a first-order startup/read bottleneck and deserves a larger grid before
  baking block-shape defaults.
- H05 EROFS capability artifact:
  `benchmarks/kvm-rootfs-format-grid/data_1.2.1780320819_x86_64_1780339221.json`
  generated a 916.1 MiB EROFS image and tried the tuned rootfs block shape, but
  the VM did not become ready. Current `guest/config/kernel/defconfig.x86_64`
  does not enable `CONFIG_EROFS_FS`, so EROFS needs a kernel-capability slice
  before it can be compared fairly. DAX remains a capability/design audit, not
  a block-image benchmark, because the current rootfs transport is virtio-blk.
- H05 EROFS kernel-capability slice landed locally: both arm64 and x86_64 guest
  defconfigs now enable `CONFIG_EROFS_FS` and `CONFIG_EROFS_FS_ZIP` so rebuilt
  kernels can mount benchmark-generated EROFS rootfs images. The next proof is
  to rebuild the x86_64 kernel asset, rerun the EROFS cell, and then include
  EROFS in the same best-vs-best rootfs-format/block-shape grid.
- H05 compression-level matrix support landed locally: the rootfs-format grid
  now accepts `--zstd-levels`, appending generated `squashfs-zstd-l<N>` variants
  such as levels 1/3/9/15/22. These variants are rebuilt from the same extracted
  rootfs and run through the same block-shape cells as production zstd,
  uncompressed SquashFS, and EROFS, so compression level is measured fairly
  rather than compared against a differently tuned block path.

## Coverage Ledger

- Unit/contract: `tests/test_archive_superseded_benchmark_artifacts.py`,
  `tests/test_benchmark_contract.py`, `tests/test_benchmark_artifacts.py`,
  `tests/test_kvm_rootfs_format_grid.py`,
  `tests/test_docker.py::TestRenderKernel::test_kernel_defconfigs_support_erofs_rootfs_experiments`,
  `cargo test -p capsem-core guest_memory_ref --lib`,
  `cargo test -p capsem-core block_guest_iovecs_reject_range_that_crosses_ram_end --lib`,
  `cargo test -p capsem-core virtio_blk --lib`,
  `cargo test -p capsem-core virtio_queue --lib`,
  `cargo test -p capsem-core virtio_mmio --lib`,
  `cargo test -p capsem-core offset_overflow_fails --lib`,
  `cargo test -p capsem-core guest_memory --lib`,
  `cargo test -p capsem-core block_data_length_overflow_returns_ioerr --lib`,
  `cargo test -p capsem-core hypervisor::kvm --lib`,
  `cargo test -p capsem-core block_read_records_queue_and_request_metrics --lib`,
  `cargo test -p capsem-core virtio_blk --lib`,
  `cargo test -p capsem-process metrics_snapshot_is_process_owned_and_versioned --bin capsem-process`,
  `cargo test -p capsem-process ipc::tests --bin capsem-process`,
  `cargo test -p capsem-service attach_metrics_snapshot_projects_security_status_fields --bin capsem-service`,
  `cargo test -p capsem-gateway fetch_status_enriches_running_vm_with_info_metrics --bin capsem-gateway`,
  `cargo test -p capsem-gateway status::tests --bin capsem-gateway`,
  `cargo test -p capsem --bin capsem format_session_resource_lines_shows_live_metrics`,
  `cargo test -p capsem --bin capsem format_session_hypervisor_lines_shows_block_counters`,
  `cargo test -p capsem --bin capsem`,
  `cargo test -p capsem-tui gateway_status_json_maps_to_tui_state --lib`,
  `cargo test -p capsem-tui stats_overlay_renders_on_demand_without_persistent_help --lib`,
  `cargo test -p capsem-tui --lib`,
  `cargo test -p capsem-proto metrics::tests --lib`,
  `cargo test -p capsem-core undo_pop_retries_last_chain --lib`,
  `cargo test -p capsem-core block_io_uring_queue_full_backpressures_without_sync_fallback --lib`,
  `cargo test -p capsem-core block_io_uring_completion_retries_backpressured_descriptor --lib`,
  `cargo test -p capsem-core block_io_uring --lib`,
  `cargo test -p capsem-service process_env_allowlist_forwards_child_runtime_knobs --bin capsem-service`,
  `python3 scripts/kvm_block_shape_grid.py --dry-run --queue-counts 1,4 --queue-sizes 128 --seg-maxes auto,64 --logical-block-sizes 512,4096`,
  `cargo test -p capsem-service attach_metrics_snapshot_projects_security_status_fields --bin capsem-service`,
  `cargo test -p capsem --bin capsem format_session_hypervisor_lines_shows_block_counters`.
- Functional: `just exec "echo ok"` passed after H01 queue activation changes.
  A live named VM smoke with `capsem info --json` passed for H03 and reported
  `metrics_schema_version=1`, `configured_ram_mb=2048`, `configured_vcpus=2`,
  host PID, host RSS, and host CPU time before the throwaway VM was deleted.
  `just exec "echo ok"` also passed after H02 made the full io_uring block
  profile the default KVM block backend.
- Adversarial: `block_guest_iovecs_reject_range_that_crosses_ram_end` proves
  a descriptor whose start GPA is valid but whose length crosses RAM end is
  rejected before raw iovecs reach host I/O. `avail_head_outside_queue_fails_closed`,
  `descriptor_next_outside_queue_fails_closed`, and
  `cycle_in_descriptor_chain_terminates` prove malformed split-ring chains fail
  closed. `zero_size_queue_operations_fail_closed` and
  `misaligned_descriptor_table_fails_closed` prove bad queue layout does not
  panic or parse misaligned descriptor memory.
  `driver_ok_rejects_ready_queue_with_zero_size` and
  `driver_ok_rejects_ready_queue_outside_guest_ram` prove malformed ready
  queues are rejected at transport activation. `guest_memory_*_offset_overflow_fails`
  tests prove hostile offset arithmetic returns errors instead of panicking.
  `block_data_length_overflow_returns_ioerr` proves aggregate descriptor length
  overflow fails the request instead of panicking.
  `block_io_uring_queue_full_backpressures_without_sync_fallback` proves a full
  io_uring submission queue does not burn CPU in the synchronous fallback path,
  does not complete the request, and can retry the same descriptor later.
  `block_io_uring_completion_retries_backpressured_descriptor` proves a real
  io_uring completion frees capacity and triggers resubmission of the rewound
  descriptor without a new guest notification.
  `block_io_uring_uses_firecracker_shaped_ring_contract` proves the io_uring
  backend comes up with a fixed registered file and ring restrictions enabled.
- E2E/VM: `just exec "echo ok"` passed for the KVM one-shot VM path. H03
  resource projection was also checked against a live named VM via
  `capsem info --json`; the second H03 live check confirmed KVM block counters
  appear in that same JSON response for a real booted VM. The third H03 live
  check confirmed gateway `/status` carries those counters to the TUI-facing
  feed for a real booted VM. H02 default async block selection was smoke-tested
  through the same KVM one-shot VM path. The latest isolated live KVM check
  used the repo assets path and confirmed the guest-visible virtio-blk geometry
  before running `capsem-bench rootfs`. The latest MQ live KVM check confirmed
  four virtio-blk queues, tuned queue size, tuned segment limit, and tuned
  logical block size in Linux sysfs before running `capsem-bench rootfs`.
- Telemetry: H03 first slice exposes existing `VmMetricsSnapshot.resources`
  fields through the service API and CLI. H03 second slice adds
  `VmMetricsSnapshot.hypervisor.block` and feeds it from the KVM virtio-blk
  backend while preserving `metrics` facade emission. H03 third slice carries
  those fields through gateway `/status` and the TUI model. H03 fourth slice
  adds OTel-compatible metric-point mapping with bounded attributes. Real OTLP
  exporter process/configuration remains open for the broader telemetry sprint.
  H02 first slice adds `async_queue_full_total` to the KVM block snapshot and
  OTel-compatible block metric points.
- Performance: canonical `just benchmark` rerun completed; benchmark artifacts
  record project version, git commit, source dirty state, host metadata, and
  active Linux x86_64 results. `scripts/compare_benchmark_artifacts.py`
  produced Linux/macOS ratios for shared lanes. Refreshed macOS artifacts from
  `1.2.1780103109` are now present on main and compared successfully. A
  canonical Linux x86_64 rerun on commit `19ca286e` recorded fresh artifacts
  for `1.2.1780320819`; it completed artifact generation but failed the
  endpoint-latency gate on service global endpoints at roughly 3-6 ms p95 and
  `/logs/{id}` at roughly 26 ms p95. The same artifact set shows Linux still
  behind macOS on the user-visible lanes: rootfs random read 1,285 vs 8,734
  IOPS, rootfs metadata 35,677 vs 199,915 stats/s, rootfs cold large-binary
  161.5 vs 977.3 MB/s, node startup 358.1 vs 77.6 ms, claude startup 1,702.2
  vs 309.0 ms, and codex startup 1,115.5 vs 237.1 ms. H02 first
  and second slices are correctness/backpressure for the io_uring path. H02
  full-profile local benchmarks measured the full async engine before grouped
  ablation: same-run rootfs showed cold binary +8.0%, small JS +2.7%, metadata
  +4.2%, but random rootfs -8.2%; same-run startup showed node +4.2% but codex
  -9.6%. Queue depth 256 was rejected after mixed ablation results. Official
  Firecracker v1.15.1 with the same Capsem rootfs/kernel workload proved the
  VMM/device path gap is real: Firecracker Sync was +46.6% random rootfs,
  +58.2% cold large-binary, +21.7% small JS, +12.1% metadata, and 12.3-42.6%
  faster on AI CLI startup. Firecracker Async remained in the same band rather
  than proving io_uring alone is the missing lever. crosvm epoll improved on
  Firecracker Sync for this workload by +10.5% random rootfs, +10.4% cold
  large-binary, +13.1% small JS, +13.6% metadata, and +6.8% codex startup, while
  crosvm corrected-uring, direct-I/O, and multi-worker ablations were rejected.
  A local
  uncommitted VirtioFS batching probe measured `/root`
  targeted disk at seq write +2.3%, seq read +2.4%, random write -0.6%, random
  read +10.8% without event-index, but it was not accepted because it was not
  the systematic backend-wide profile now being pursued.
  The H05 rootfs-only block-shape canonical run on commit `b834d554` improved
  the committed Linux baseline on the lanes the focused grid predicted:
  rootfs random read 2,686 IOPS (+109.1%), small-JS reads 88,791 ops/s
  (+18.7%), metadata stat 58,674 stats/s (+64.5%), python startup 31.7 ms
  (+27.8% faster), node 302.1 ms (+18.5%), claude 1,509.7 ms (+12.8%),
  gemini 3,206.5 ms (+1.4%), and codex 979.0 ms (+13.9%). It did not close
  the macOS gap: Linux/macOS ratios were rootfs seq 0.17x, random IOPS 0.31x,
  cold large-binary 0.17x, small-JS 0.22x, metadata 0.29x, disk seq read
  0.08x, and disk random read 0.08x. Against the same Linux host-native/raw
  artifact, the VM measured disk seq write 0.35x host, disk seq read 0.05x,
  disk random read 0.02x, small-file reads 0.52x, and metadata stats 0.27x.
  VM random write appeared 4.03x host-native because the two paths have
  different buffering/sync behavior, so that lane is not a clean raw-efficiency
  signal.
- Missing/deferred: Real OTLP exporter process/configuration is deferred to the
  broader telemetry sprint. Endpoint-latency regressions are recorded by the
  canonical benchmark gate and still need a control-plane performance fix.
