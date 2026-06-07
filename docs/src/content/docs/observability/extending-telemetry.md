---
title: Extending Telemetry
description: How engines, rule packs, and plugins add telemetry without breaking the event contract.
sidebar:
  order: 2
---

New telemetry starts with normalized events, not ad hoc metrics.

## Order Of Operations

1. Define or extend the normalized Security Event subject.
2. Emit a resolved security event with attribution, decision, findings, and
   evidence.
3. Update typed VM/host accumulators from the resolved event.
4. Expose bounded summaries in status/debug/UI.
5. Export low-cardinality OpenTelemetry metrics.

The canonical event journal is the source of truth. Domain tables and UI views
are projections.

## Attribution

Every event should carry the relevant `vm_id`, `session_id`, `profile_id`,
`user_id`, trace id, and accounting owner. Accounting owner matters: host AI
work is not VM spend even when it is correlated with a VM.

## Detection And Enforcement

Detection runs before audit logging and telemetry sinks so the emitted event
already includes findings. Enforcement decisions and declarative mutations are
recorded before transport projection maps them to continue/rewrite/stop.

## Plugins

Future plugins receive and return deterministic `SecurityEvent` values. They
must not depend on ambient filesystem, network, clock, process state, or hidden
runtime state. If a plugin needs history, Capsem embeds the trace/history
snapshot in the event. The invariant is:

```text
same plugin hash + same input event hash = same output event hash
```

This supports replay, auditability, deterministic tests, and signed plugin
bundles.

