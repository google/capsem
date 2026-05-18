# Profile V2 S07 UDS API

## Goal

Start S07 on the cleaned Profile V2 line by landing the typed UDS foundation
without bundling the full public API surface into one giant change.

## First Slice

Add the S07/S12 foundational metrics IPC contract:

- `capsem_proto::metrics` shared snapshot structs.
- `ServiceToProcess::GetMetricsSnapshot`.
- `ProcessToService::MetricsSnapshot`.
- Process-side handling that returns a bounded default snapshot until the S12
  accumulator lands.
- Contract tests for serde/bincode roundtrips and process dispatch
  classification.

## Later S07 Slices

- Profile list/get/create/fork/update/delete/resolve route group.
- Dedicated Rules API list/get/add/remove/evaluate route group.
- Confirm pending listing shape, leaving resolution to S15.
- Skills list/add/delete route group.
- MCP route shape cleanup in the new model where current routes are not enough.
- Debug report/API provenance updates for all UDS-visible surfaces.

## Done For This Slice

- Focused proto/process tests pass.
- Service/process code compiles with the new variants.
- Changelog and Profile V2 trackers name the partial S07 progress and remaining
  release holds.

## Testing Proof Matrix

- Unit/contract: capsem-proto metrics/IPC roundtrips.
- Functional: capsem-process IPC dispatch recognizes the metrics request and
  responds with a snapshot shape.
- Adversarial: bincode test proves the real IPC wire format carries the new
  nested snapshot.
- E2E/VM: not required for the proto foundation slice; later S07 API routes need
  service-level integration and eventually VM proof.
- Telemetry: no live counters yet; S12 accumulator remains deferred.
- Performance: snapshot request is classified as a health/read-only IPC action,
  not a job or lifecycle mutation.
