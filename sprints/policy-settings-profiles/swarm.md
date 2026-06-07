# Policy Settings Profiles Swarm

## Purpose And Rules

This swarm captures parallel investigation for S08+ policy/runtime work. Agents
must produce durable findings before we turn their output into sprint changes.

Rules:
- No implementation edits unless the main thread explicitly assigns a worker.
- Read this board first, then the assigned finding doc.
- Keep findings tied to exact files, sprint targets, and required proof.
- Do not mark the swarm complete while any active agent is uncaptured.

## Status Legend

- `Queued`: finding doc exists, agent not launched.
- `Active`: agent launched, output not yet captured.
- `Captured`: agent completed and findings copied into the finding doc.
- `Closed`: findings synthesized into tracker/MASTER/sprint docs.

## Finding Docs Index

| Status | Domain | Agent | Finding Doc | Sprint Targets |
| --- | --- | --- | --- | --- |
| Captured | CEL authoring namespace | 019e4c17-b86d-7403-8ff7-ec70bdbac487 / Herschel | [swarm-findings/cel-authoring-namespace.md](swarm-findings/cel-authoring-namespace.md) | S08b, S08c, S09, S14, S19 |
| Captured | Policy context proto contract | 019e4c1e-cedd-76b1-b843-0c83d1aad218 / Mencius | [swarm-findings/policy-context-proto.md](swarm-findings/policy-context-proto.md) | S08b, S08c, S09, S14, S19 |

## Resume Protocol

1. Check the Finding Docs Index for any `Active` rows.
2. Poll the listed agent ids before launching replacement work.
3. Capture completed output into the matching finding doc.
4. Update this board, then update `tracker.md` and affected sprint docs.

## Required Finding Shape

Each finding must include:
- Severity: P0, P1, P2, or P3.
- Release or user impact.
- Exact paths and line anchors when known.
- Owning sprint task IDs or proposed task IDs.
- Required code/test proof.
- Required CLI, UI, docs, telemetry, VM, or performance proof when relevant.
- Transfer status.

## Completed Agents

- 019e4c17-b86d-7403-8ff7-ec70bdbac487 / Herschel: canonical CEL namespace
  investigation captured in
  [swarm-findings/cel-authoring-namespace.md](swarm-findings/cel-authoring-namespace.md).
- 019e4c1e-cedd-76b1-b843-0c83d1aad218 / Mencius: first `capsem-proto`
  policy context schema captured in
  [swarm-findings/policy-context-proto.md](swarm-findings/policy-context-proto.md).

## Active Agents

None.

## Launch Queue

1. CEL authoring namespace: investigate how to expose canonical rule authoring
   roots such as `http.request.host.contains("google")` and
   `http.request.header("authorization").exists()` without exposing `event.*`
   in rule source. `SecurityEvent` may remain the internal audit envelope, but
   the public rule ABI is the canonical policy context.
2. Policy context proto contract: implement the shared typed policy context
   schema in `capsem-proto` with current Rust names, an explicit schema version
   field, no `V1` type suffixes, no CEL dependency, and direct tests for the
   high-level DSL/CEL mirror surface.

## Intake Checklist

- [x] Agent output copied into finding doc.
- [x] Finding doc status changed to `completed`.
- [x] Finding Docs Index updated.
- [x] P0/P1 findings deduplicated.
- [x] Tracker/MASTER/sprint docs updated with accepted work.

## Completeness Gate

- [x] No `Active` rows remain.
- [x] No finding doc contains `Awaiting agent output`.
- [x] Every completed doc has findings or explicitly says no blockers found.
- [x] Every P0/P1 has an owner and proof target.
