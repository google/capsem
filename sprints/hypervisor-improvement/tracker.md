# Sprint: hypervisor-improvement

## Tasks

- [x] Create meta-sprint structure and sub-sprint plan.
- [ ] H00: close current KVM/block context and benchmark truth.
- [x] H00: make benchmark artifact retention part of `just benchmark`.
- [ ] H01: safety and queue contracts.
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

## Coverage Ledger

- Unit/contract: `tests/test_archive_superseded_benchmark_artifacts.py`,
  `tests/test_benchmark_contract.py`, `tests/test_benchmark_artifacts.py`.
- Functional: pending per sub-sprint.
- Adversarial: pending per sub-sprint.
- E2E/VM: pending per sub-sprint.
- Telemetry: pending per sub-sprint.
- Performance: pending per sub-sprint.
- Missing/deferred: implementation has not started; this commit is planning
  only.
