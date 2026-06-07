# H04 - CPU SMP And Lifecycle

## Goal

Make Capsem's vCPU lifecycle and SMP behavior boring, observable, and
reproducible.

## Scope

- Add or evaluate `KVM_RUN.immediate_exit` for pause/stop kicks.
- Add vCPU exit counters and latency buckets.
- Tighten SMP topology/CPUID presentation and document current limits.
- Audit AP startup, HLT, `NotReady`, fail-entry, PIT/timer behavior, and sleeps
  in the run loop.
- Consider Firecracker-style paused bootstrap only if it buys deterministic
  setup or cleaner suspend/resume.
- Keep suspend/resume process-continuity tests as a long-term contract.

## Done

- SMP-visible CPU count and topology are intentional.
- Pause/resume/stop are deterministic under load.
- Status and telemetry can explain vCPU behavior during real workloads.

## Proof

- Focused vCPU lifecycle tests.
- `capsem-doctor` SMP checks.
- suspend/resume process-continuity check after lifecycle changes.

