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

- Profile list/get/create/fork/update/delete/resolve route group. The first
  route slice is read-only list/get/resolve.
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

## Second Slice

Add read-only service profile routes:

- `GET /profiles`
- `GET /profiles/{id}`
- `GET /profiles/{id}/effective`

This gives clients a stable discovery/resolve API before S07 mutation routes
land.

## Third Slice

Add profile mutation routes:

- `POST /profiles`
- `POST /profiles/{id}/fork`
- `PUT /profiles/{id}`
- `DELETE /profiles/{id}`

This completes the dedicated profile CRUD/fork group at the service UDS layer.

## Fourth Slice

Add the read-only and dry-run half of the dedicated Rules API:

- `GET /rules?profile=<id>&callback=<type>`
- `GET /rules/{rule_id}`
- `POST /rules/evaluate`

The list/get routes expose resolved Profile V2 rules with canonical
`security.rules.<type>.<name>` ids, provenance, ownership metadata, source
profile, priority, and match condition. The evaluate route runs the V2 policy
engine against a synthetic JSON subject and returns the matched rule, decision,
`would_ask`, and reason without enforcing, prompting, or writing telemetry.

Rule create/delete remains a later S07 slice so this change can lock the query
and evaluator contract first.

## Rules API Hardening

Before mirroring the Rules API through S08 HTTP gateway routes, harden the UDS
contract with:

- A functional workflow that chains profile create, profile list, rule list,
  rule get, dry-run evaluate, profile update, and dry-run re-evaluate.
- Generated-rule dry-run coverage for `http.read` and `http.write` catch-all
  callbacks.
- Shared Policy V2 support for `http.read` / `http.write` callback names and
  boolean `true` / `false` CEL terms used by generated rules.
- A bounded large-profile evaluation test so the gateway does not inherit an
  obviously unbounded hot path.

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
- Rules API performance: `POST /rules/evaluate` is covered by a 32-iteration
  large-profile regression test over 161 HTTP rules with a 1.5s debug-build
  budget.
