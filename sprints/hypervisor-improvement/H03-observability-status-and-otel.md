# H03 - Observability Status And OTel

## Goal

Expose the hypervisor as a user-understandable system, not a black box. Counters
must be useful in status/debug output and stable enough for OpenTelemetry.

## Scope

- Inventory current status/info paths and metrics recorder/exporter surfaces.
- Add stable low-cardinality metrics for:
  - vCPU count, exits, pause/resume events, `KVM_RUN` failures;
  - CPU usage by VM/process where host APIs allow it;
  - memory configured, resident set, guest RAM size, and snapshot/checkpoint
    bytes;
  - block request counts, bytes, latency, queue depth, backend, fallbacks,
    queue-full/backpressure, interrupts raised/suppressed;
  - VirtioFS request counts, bytes, latency, errors, queue wakes, FUSE limits;
  - vsock queue/call/kick activity and reconnects;
  - boot, suspend, resume, quiesce, checkpoint, restore timing.
- Surface summarized resource and hypervisor counters in `capsem status`,
  `capsem info`, MCP status tools, and docs where appropriate.
- Keep hot-path labels bounded: VM id/session id may belong in spans/logs, not
  high-cardinality metric labels.

## OTel Contract

- Metric names are stable and namespaced.
- Units are described.
- Labels are bounded and documented.
- Status output uses the same source of truth as metrics/export when possible.
- Missing platform-specific values are displayed as unavailable, not silently
  zero.

## Done

- A real VM run shows CPU, memory, I/O, and hypervisor counters in user-facing
  status.
- The same counters are available through the metrics facade for future or
  current OTLP export.

## Proof

- Unit tests with a debug metrics recorder.
- Status/info functional tests.
- Real-session inspection after `just exec "echo ok"` and a live
  `capsem info --json` check.

## Progress

- First slice complete: service `/info` and `capsem info` now expose the
  existing live `VmMetricsSnapshot.resources` fields for configured RAM/vCPUs,
  host PID/RSS/CPU time/CPU percent, and disk counters when available.
- Next slice: wire queue/backend/hypervisor counters into status and the
  OTel/exporter-facing metrics surface with bounded labels.
