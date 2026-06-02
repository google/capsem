# Hypervisor Improvement Plan

This plan is the working entry point for implementation. See `MASTER.md` for
the meta-sprint structure.

## What We Are Building

A systematic hypervisor improvement program based on the Firecracker audit:
correctness first, then event delivery and lifecycle, then user-visible
observability and performance proof.

## Key Decisions

- Do not make io_uring default until safety, backpressure, and benchmarks prove
  it for a specific device/workload.
- Preserve Capsem's cross-platform product storage model.
- Treat metrics/status/OTel as foundation work, not decoration.
- Commit at functional milestones with changelog entries and proof.

## Ordering

Recommended:

1. H00 reality and wrap-up.
2. H01 safety and queue contracts.
3. H03 observability/status/OTel.
4. H02 event delivery/backpressure.
5. H04 CPU/SMP lifecycle.
6. H05 storage/rootfs experiments.
7. H08 disk throughput attribution for the remaining raw-throughput gap.
8. H06/H07 proof, docs, release gate.

Reasoning: safety must come before exposing official counters; counters must
come before aggressive optimization so we can explain real workload behavior.

## Files Likely To Change Later

- `crates/capsem-core/src/hypervisor/kvm/`
- `crates/capsem-core/src/vm/`
- `crates/capsem-service/`
- `crates/capsem-mcp/`
- `guest/artifacts/capsem_bench/`
- `guest/artifacts/diagnostics/`
- `scripts/compare_benchmark_artifacts.py`
- `docs/src/content/docs/development/benchmarking.md`
- `docs/src/content/docs/observability/`
- `skills/dev-benchmark/SKILL.md`
- `skills/dev-testing-hypervisor/SKILL.md`
- `CHANGELOG.md`

## Done

- Hypervisor safety contracts are stricter and tested.
- CPU, memory, I/O, queue, and lifecycle counters are visible in status and
  OTel-ready metrics.
- KVM event delivery and async backpressure have Firecracker-grade contracts
  where implemented.
- Storage/rootfs experiments have Linux and Apple-compatible benchmark proof.
- Remaining Linux disk throughput gaps are attributed by transport path before
  implementation changes are accepted.
- Docs and sprint tracker carry enough context for future work without relying
  on chat history.
