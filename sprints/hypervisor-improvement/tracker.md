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
  - [ ] Extend the same backpressure/event-loop audit to other KVM devices and
        completion paths.
- [ ] H04: CPU, SMP, and lifecycle.
- [ ] H05: storage, rootfs, and filesystem experiments.
- [ ] H06: benchmark and product proof.
- [ ] H07: docs, changelog, release gate.

## Notes

- User priority: improvements should include core systematic goodness, not only
  benchmark-visible wins.
- User priority: counters must become visible in status and available for
  OpenTelemetry.
- User priority: expose CPU usage, I/O, and memory usage so users get a clear
  system view.
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

## Coverage Ledger

- Unit/contract: `tests/test_archive_superseded_benchmark_artifacts.py`,
  `tests/test_benchmark_contract.py`, `tests/test_benchmark_artifacts.py`,
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
  `cargo test -p capsem-service attach_metrics_snapshot_projects_security_status_fields --bin capsem-service`,
  `cargo test -p capsem --bin capsem format_session_hypervisor_lines_shows_block_counters`.
- Functional: `just exec "echo ok"` passed after H01 queue activation changes.
  A live named VM smoke with `capsem info --json` passed for H03 and reported
  `metrics_schema_version=1`, `configured_ram_mb=2048`, `configured_vcpus=2`,
  host PID, host RSS, and host CPU time before the throwaway VM was deleted.
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
- E2E/VM: `just exec "echo ok"` passed for the KVM one-shot VM path. H03
  resource projection was also checked against a live named VM via
  `capsem info --json`; the second H03 live check confirmed KVM block counters
  appear in that same JSON response for a real booted VM. The third H03 live
  check confirmed gateway `/status` carries those counters to the TUI-facing
  feed for a real booted VM.
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
  `1.2.1780103109` are now present on main and compared successfully. H02 first
  and second slices are correctness/backpressure for the opt-in io_uring path;
  no canonical benchmark was rerun because the default block backend is
  unchanged.
- Missing/deferred: Real OTLP exporter process/configuration is deferred to the
  broader telemetry sprint; full benchmark rerun is deferred until a
  performance-affecting H02/H03 milestone lands.
