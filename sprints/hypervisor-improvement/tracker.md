# Sprint: hypervisor-improvement

## Tasks

- [x] Create meta-sprint structure and sub-sprint plan.
- [x] H00: close current KVM/block context and benchmark truth.
- [x] H00: make benchmark artifact retention part of `just benchmark`.
- [ ] H01: safety and queue contracts.
  - [x] Record main merge and refreshed macOS benchmark comparison baseline.
  - [x] Add full guest-memory range validation before raw host pointers.
  - [x] Reject malformed virtqueue descriptor indices and cycles.
- [ ] H03: observability, status, and OTel resource counters.
- [ ] H02: event delivery and backpressure.
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

## Coverage Ledger

- Unit/contract: `tests/test_archive_superseded_benchmark_artifacts.py`,
  `tests/test_benchmark_contract.py`, `tests/test_benchmark_artifacts.py`,
  `cargo test -p capsem-core guest_memory_ref --lib`,
  `cargo test -p capsem-core block_guest_iovecs_reject_range_that_crosses_ram_end --lib`,
  `cargo test -p capsem-core virtio_blk --lib`,
  `cargo test -p capsem-core virtio_queue --lib`.
- Functional: pending per sub-sprint.
- Adversarial: `block_guest_iovecs_reject_range_that_crosses_ram_end` proves
  a descriptor whose start GPA is valid but whose length crosses RAM end is
  rejected before raw iovecs reach host I/O. `avail_head_outside_queue_fails_closed`,
  `descriptor_next_outside_queue_fails_closed`, and
  `cycle_in_descriptor_chain_terminates` prove malformed split-ring chains fail
  closed.
- E2E/VM: pending per sub-sprint.
- Telemetry: pending per sub-sprint.
- Performance: canonical `just benchmark` rerun completed; benchmark artifacts
  record project version, git commit, source dirty state, host metadata, and
  active Linux x86_64 results. `scripts/compare_benchmark_artifacts.py`
  produced Linux/macOS ratios for shared lanes. Refreshed macOS artifacts from
  `1.2.1780103109` are now present on main and compared successfully.
- Missing/deferred: H01 implementation has started; functional VM smoke,
  telemetry/status, and full benchmark reruns are deferred until a functional
  H01 milestone lands.
