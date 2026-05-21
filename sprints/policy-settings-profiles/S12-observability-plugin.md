# S12 - OpenTelemetry Metrics Architecture

Last updated: 2026-05-15

## Why This Exists

Inherits the release-team handoff "OpenTelemetry Metrics Handoff"
(2026-05-15). During release-debug-loop final verification the release
branch found that service `/list` was opening every running VM's
`session.db` to compute aggregate telemetry. That coupled list/status
to per-session SQLite at exactly the wrong time. The release branch
shipped a narrow hotfix:

- `/list` no longer reads `session.db`.
- `/list` keeps only in-memory service state plus a placeholder hook for
  future live metrics (`attach_list_live_metrics_placeholder()` with
  `FIXME(otel-sprint)`).
- Single-VM/detail paths may still read durable session DB telemetry
  until this sprint replaces them with live snapshots where appropriate.
- SQLite remains the durable forensic/audit store, not the hot status
  source.

This sprint is the real metrics/OTel sprint that the release team
deferred to. It must not regress the hotfix.

## Goal

Replace ad-hoc telemetry with a typed live-metrics architecture in which
the **in-memory per-VM accumulator in `capsem-process` is the only
source of truth at runtime**. `session.db` becomes a write-only durable
mirror from the runtime's perspective and is read exactly twice in a
VM's life:

1. At VM launch in `capsem-process`, to seed the accumulator with prior
   cumulative totals for persistent VMs.
2. Post-mortem (the VM's process is gone) for forensics, support
   bundles, and the stopped-VM `/info` fallback.

Everything else -- `/list`, `/info` on a running VM, gateway status,
UI polling, scrape endpoints -- reads memory only.

VM status health is a live point-in-time view. Running VM status surfaces read
the in-memory accumulator. Persistent VMs seed/recompute cumulative totals from
`session.db` exactly once at process load, then continue from memory. This is
the agreed model for cost, model call count, provider/model usage, enforcement,
detection, and activity health: accurate enough for live operations without
reopening SQLite on status fan-out paths.

## Dependency On S08a

[S08a - Rule Abstraction And Detection Architecture](S08a-rule-abstraction-detection-architecture.md)
settled the first contract slice: enforcement and detection are separate
profile-owned rule families, detections emit typed findings on
`ResolvedSecurityEvent`, and OTel labels must stay bounded. S12 can freeze the
no-SQL runtime accumulator contract, but metric implementation must consume the
S08a/S08b finding schema rather than inventing a parallel detection model.

Post-regroup terminology: S08b owns the runtime `/enforcement/*` and
`/detection/*` route groups. S12 exports their live health, counters, and match
stats; it does not create a third generic "rules" metrics family. Full match
evidence stays in the resolved-event journal and local backtest/hunt responses,
not in OpenTelemetry labels.

## Single Source Of Truth

```
VM launch in capsem-process:
  if persistent VM and session.db exists:
    seed_accumulator_from_session_db(session_dir)
      -> open session.db once
      -> run cumulative aggregate queries (the same queries the release
         branch isolated in enrich_telemetry_from_session_db)
      -> populate VmMetricsAccumulator with the durable totals
  else:
    VmMetricsAccumulator starts at zero

VM runtime, for the entire lifetime of the process:
  every observable event
    -> increment in-memory VmMetricsAccumulator
    -> append the event to session.db via existing DbWriter (async batched)

  snapshot RPC (ServiceToProcess::GetMetricsSnapshot)
    -> read the accumulator only
    -> never re-open session.db
```

Consequences:

- A running VM's counters always carry cumulative totals across restarts
  for persistent VMs, without any caller having to opt in.
- `session.db` reads on the runtime data path: **zero** after boot.
- Crash recovery: on a clean shutdown drain the DbWriter queue; on a
  crash the durable totals lag behind memory by at most one DbWriter
  batch. Document that bound; do not engineer around it (the
  alternative is synchronous writes on the hot path, which we are
  explicitly rejecting).
- `enrich_telemetry_from_session_db()` (the release-branch helper on
  the service side) is **deleted from `capsem-service`**. Its logic
  moves into `capsem-process` boot as
  `seed_accumulator_from_session_db()`. Service stops opening
  `session.db` entirely.

## Surfaces

| Endpoint | Source | Cost |
| --- | --- | --- |
| `GET /list` | in-memory service state + IPC snapshot per running VM | bounded, no SQL, release hotfix preserved |
| `GET /info/{id}` (running VM) | IPC snapshot from capsem-process accumulator | one bounded IPC round-trip, no SQL |
| `GET /info/{id}` (stopped VM) | one-shot session.db read | cold path, never on a UI poll loop because the VM is gone |
| `GET /metrics/json` | accumulator snapshots aggregated across running VMs | bounded, no SQL |
| `GET /metrics` (Prometheus/OTel scrape) | same accumulators, low-cardinality labels | bounded, no SQL |
| Forensic / support-bundle / `inspect-session` tools | session.db directly | unchanged |
| Gateway `/status`, `/metrics/json`, `/metrics` | proxy of the service surfaces | unchanged plumbing |

There is no `/info/{id}/history` endpoint and no `?history=true` query
param. The single endpoint answers correctly for both running and
stopped cases because the accumulator was already seeded with durable
totals at boot.

## Hard Constraints

- **No SQL on hot fan-out paths.** `/list`, `/info` on a running VM,
  `/metrics/json`, `/metrics`, and gateway status routes never open
  `session.db`. The release-branch regression test stays green.
- **`session.db` reads are localized.** Only `seed_accumulator_from_session_db`
  (capsem-process, at boot) and the stopped-VM `/info` fallback (one-shot,
  cold) read the durable store from the runtime side.
- **IPC plane separation.** Live metrics travel on `capsem-proto::ipc`
  (bincode over the per-VM UDS), not the rmp-serde host-guest plane.
- **Exported labels stay low-cardinality.** No raw paths, URLs, prompts,
  commands, or error strings as Prometheus/OTel labels. High-cardinality
  detail goes into bounded JSON top-N summaries.
- **Do not synthesize guest metrics.** If only host-side data is
  available, name it host-side (e.g. `host_process_rss_bytes`). Wait
  for the guest agent / hypervisor path before claiming guest internals.

## Shared Proto Contract

Add `capsem-proto::metrics` (new module). Sketch, not final API:

```rust
pub struct VmMetricsSnapshot {
    pub schema_version: u32,
    pub vm_id: String,
    pub persistent: bool,
    pub lifecycle: VmLifecycleMetrics,
    pub resources: VmResourceMetrics,
    pub enforcement: VmEnforcementMetrics,
    pub http: VmHttpMetrics,
    pub dns: VmDnsMetrics,
    pub model: VmModelMetrics,
    pub detection: VmDetectionMetrics,
    pub forward_plugin: VmForwardPluginMetrics,
    pub mcp: VmMcpMetrics,
    pub filesystem: VmFilesystemMetrics,
    pub captured_at_unix_ms: u64,
}
```

Principles:

- Contracts live in proto first. Service/process/gateway/CLI/frontend
  consume the same Rust types where possible.
- Counters are monotonic.
- Exported metric names use low-cardinality labels only.
- High-cardinality details stay in JSON summaries, bounded by top-N.
- Pass rates / ratios are derived from counters in renderers, not
  stored separately.

## Metric Taxonomy

### Ask / Enforcement

The `ask` event boundary is owned by S06-pre. Its definition for this
sprint:

**An "ask" is any enforcement callback that matched a rule with
`decision = "ask"` (set in profile or admin TOML) and was resolved through the
`Confirmer` trait into a final `Decision::Accept | Deny`.**

The durable source of truth is the `policy_confirm_events` table
(landing in S06-pre slice 7). The accumulator increments on the same
event boundary and is seeded at boot from cumulative counts in the
table.

Counters:

- `total_asks`
- `asks_allowed` (resolved `accept`)
- `asks_denied` (resolved `deny`)
- `asks_errored` (confirmer returned an error, timed out, or panicked)

Derived (in renderer, not stored):

- ask pass rate = `asks_allowed / total_asks`
- ask denial rate = `asks_denied / total_asks`
- ask error rate = `asks_errored / total_asks`

**Decision on `asks_warned`:** the release-team draft listed five
buckets including `asks_warned`. The Confirmer trait in S06-pre is
binary (`Accept | Deny`). The legacy MCP `ToolDecision::Warn` concept
("notify the user but proceed unless they object") is a UX concern at
the user-facing surface, not a third policy outcome at the engine
boundary -- the engine still resolves to allow or deny.

This sprint therefore **drops `asks_warned` from the enforcement ask metric
family**. The "warn" UX retains a separate home in MCP-specific
tool-invocation metrics (see MCP section: `mcp_tool_invocations_warned_total`)
where the legacy ToolDecision::Warn path lives. The enforcement ask family
stays clean and binary.

Enforcement rule counters:

- `enforcement_events_evaluated_total`
- `enforcement_rule_matches_total`
- `enforcement_decisions_allowed_total`
- `enforcement_decisions_denied_total`
- `enforcement_decisions_asked_total`
- `enforcement_decisions_rewritten_total`
- `enforcement_errors_total`

JSON-only summaries include bounded top-N rule ids/pack ids by match count,
recent typed failure reasons, and last-match timestamps. Rule ids are allowed in
bounded JSON summaries and service status payloads; exported metrics should keep
labels to profile id/revision, VM id, event family, decision, and coarse rule
origin/scope unless S08b defines a bounded registry label set.

### HTTP / HTTPS

Avoid `net_*` for user-facing metric names; the MITM/user-visible path
is HTTP/HTTPS.

Counters:

- `http_requests_total`
- `http_requests_allowed_total`
- `http_requests_warned_total`
- `http_requests_denied_total`
- `http_requests_errored_total`
- `http_bytes_sent_total`
- `http_bytes_received_total`

JSON-only summaries (bounded top-N):

- top blocked domains
- top upstream status classes
- recent denial reasons

### DNS

Separate from HTTP.

Counters:

- `dns_queries_total`
- `dns_queries_allowed_total`
- `dns_queries_warned_total`
- `dns_queries_denied_total`
- `dns_queries_rewritten_total`
- `dns_queries_errored_total`

### MCP

Split `tool_calls` and `mcp_calls` by plane.

Counters (server-level aggregation in exported metrics; tool-level in
JSON top-N):

- `mcp_tool_invocations_total`
- `mcp_tool_invocations_allowed_total`
- `mcp_tool_invocations_warned_total`  (legacy ToolDecision::Warn lives here)
- `mcp_tool_invocations_denied_total`
- `mcp_tool_invocations_errored_total`
- `mcp_servers_connected_total`
- `mcp_servers_disconnected_total`
- `mcp_server_errors_total`

### Filesystem

Counters (replace coarse `file_events`):

- `fs_reads_total`
- `fs_writes_total`
- `fs_creates_total`
- `fs_deletes_total`
- `fs_restores_total`
- `fs_errors_total`
- `fs_bytes_read_total`
- `fs_bytes_written_total`

Low-cardinality `scope` label (`workspace | system | session`) is
acceptable. No raw paths.

### Model

Counters:

- `model_requests_total`
- `model_requests_allowed_total`
- `model_requests_warned_total`
- `model_requests_denied_total`
- `model_requests_errored_total`
- `model_calls_total` (alias/compat rendering may point at
  `model_requests_total`, but the health vocabulary must expose "model call
  count" clearly)
- `model_input_tokens_total`
- `model_output_tokens_total`
- `model_estimated_cost_micros_total`  (integer micros; floating-point
  cost is renderer-only)

Bounded JSON-only summaries:

- calls by provider;
- calls by model;
- token totals by provider/model;
- estimated cost by provider/model in integer micros;
- recent model errors without raw prompt/error-string labels.

Provider and model may be OTel labels only when sourced from the VM-effective
profile/provider registry and bounded to configured values. Unknown or
unconfigured values collapse to `unknown` / JSON-only detail. Raw prompts,
request bodies, and error strings are never labels.

### Detection / Findings

S08a chooses `DetectionFinding` as the metric input and S08b owns
resolved-event emission. S12 consumes those findings through the live
accumulator:

- `detection_events_evaluated_total`
- `detection_findings_total`
- `detection_findings_by_severity_total`
- `detection_rule_matches_total`
- `detection_hunt_queries_total`
- `detection_backtest_queries_total`
- `detection_errors_total`

Exported labels stay low-cardinality: profile id/revision, VM id, detection
pack id, severity, status, and event family where bounded. Rule names,
free-form finding text, paths, URLs, prompts, commands, and raw model errors
stay in bounded JSON summaries or the resolved-event store. Provider/model/cost
metrics follow the same rule: provider and normalized model family may be
labels only after capping/normalization; raw model strings and prompts are not
labels.

Backtest/hunt responses can return full local evidence through the owning API,
but OTel only exports aggregate counters and bounded summaries. The live
accumulator records runtime match totals; S08b/S08c own historical backtest
correctness and evidence diversity.

### Centralized Forward Plugin

S13's forward plugin is reflected in VM health and OTel without becoming a
separate security truth source. It reports health and attribution for decisions
or observer exports that already flow through the resolved-event pipeline.

Counters:

- `forward_plugin_decision_requests_total`
- `forward_plugin_decision_allows_total`
- `forward_plugin_decision_denies_total`
- `forward_plugin_decision_asks_total`
- `forward_plugin_observed_events_total`
- `forward_plugin_observed_findings_total`
- `forward_plugin_errors_total`
- `forward_plugin_timeouts_total`

JSON-only summaries include endpoint health, last successful exchange time,
last typed error, and bounded outcome summaries. Exported labels stay bounded:
profile id/revision, VM id, plugin mode (`decision | observer`), outcome, and
coarse error class. Raw endpoint URLs, auth details, event evidence, and
unbounded error strings are never labels.

### Resources

Available now (host-side, deterministic):

- configured RAM MB
- configured vCPU count
- host process PID
- `host_process_rss_bytes` (named host-side -- not guest memory)
- host process CPU time / percentage
- session / workspace / rootfs-overlay byte usage from filesystem
  metadata

Later sources (when available):

- guest-reported memory pressure/used/free via guest agent
- guest-reported disk IO and network IO
- hypervisor-specific counters if Apple VZ / Linux KVM backends expose
  them through our abstraction (see Open Questions)

If the hypervisor API does not expose guest memory directly, do not
synthesize it. Use host process RSS with an explicit name.

### Lifecycle

- current lifecycle state
- uptime seconds
- boot count
- restart count
- suspend count
- resume count
- shutdown count
- unexpected exit count
- last transition timestamp
- last error (JSON only, never an exported label)

## Implementation Plan

Sprint size note: this is a wide sprint that touches proto, process,
service, gateway, frontend, and durable-store policy. Phases A-F
below are written as the work units. If once we start phase B/C the
scope feels unwieldy in one branch, we will split this into S12a-S12e
sub-sprints (proto + accumulator + seed; service /list + /info;
service /metrics endpoints; gateway + UI; durable cleanup), the same
way S06-pre got sub-slices. The decision is deferred until execution
starts.

### Phase A -- Proto contracts (foundational; lands inside or alongside S07)

- Add `capsem_proto::metrics` structs.
- Add `ServiceToProcess::GetMetricsSnapshot` and
  `ProcessToService::MetricsSnapshot` variants in `capsem-proto::ipc`.
- Bincode roundtrip tests.
- Schema/version compatibility tests.

### Phase B -- Process accumulator + boot seed

- Add `VmMetricsAccumulator` (in-memory, per-VM).
- Add `seed_accumulator_from_session_db(session_dir)` invoked exactly
  once at VM launch for persistent VMs.
- Update the accumulator at the same event points that currently write
  durable telemetry. Tests do not require a DbWriter or SQLite to
  exercise accumulator math.
- Snapshot is a bounded copy; never blocks VM control flow.

### Phase C -- Service integration

- Add bounded-time `request_metrics_snapshot(vm_id)` helper.
- Wire it into `/list` without holding the instances lock across IPC.
- Wire it into `/info/{id}` for running VMs (no SQL).
- Stopped-VM `/info/{id}` falls back to a one-shot `session.db` open
  by VM id (cold path, single VM, not a fan-out).
- **Delete `enrich_telemetry_from_session_db()`** from
  `capsem-service`. Logic moved to `capsem-process` boot.
- Add `/metrics/json` typed response.
- Add `/metrics` scrape response with low-cardinality labels only.

### Phase D -- Gateway integration

- Proxy / render service typed metrics JSON.
- Tests that client/UI metrics survive gateway translation.
- Decide: gateway renders Prometheus text or proxies service `/metrics`.

### Phase E -- Client/UI integration

- Consume typed metrics JSON for VM cards/status panels.
- Render ask pass rate from counters in the UI, not server-side.
- Render enforcement and detection match stats from typed live metrics, while
  linking into S08b/S08c backtest/hunt surfaces for event-level evidence.
- Render model call count, provider/model usage, token counts, and estimated
  cost from typed live metrics. The UI may format cost, but it must not invent
  cost values absent from the accumulator.
- Render detection finding health from typed metrics once S08a/S08b land.
- Resource labels distinguish configured / host-side / guest-side
  sources.

### Phase F -- Durable DB cleanup

- Remove any remaining SQL reads from hot status/list paths
  (post-seed, the only `session.db` reads on the runtime side should
  be the one at boot and the stopped-VM `/info` fallback).
- `inspect-session` and support-bundle tooling continue to read
  `session.db` directly; that is intentional.

## Testing Plan

### Proto

- Bincode roundtrip for new service/process IPC variants.
- Serialization compatibility for `VmMetricsSnapshot`.
- Schema version test.

### Process

- Unit tests for ask counters and pass-rate inputs.
- Unit tests for enforcement decision and match counters.
- Unit tests for HTTP/DNS/model/MCP/filesystem/resource accumulator
  updates without `DbWriter`/SQLite.
- Unit tests for model health counters and bounded provider/model summaries,
  including unknown provider/model collapse and integer-micros cost handling.
- Unit tests for detection metric counters once S08a/S08b provide the finding
  schema.
- Unit tests for forward-plugin health counters and bounded summaries.
- `seed_accumulator_from_session_db` test: persistent VM with an
  existing session.db starts with the durable totals; ephemeral VM or
  missing DB starts at zero.
- Crash bound test: events written but not yet flushed are lost on
  abrupt termination; the documented bound is at most one DbWriter
  batch.

### Service

- Mock IPC test for `/list` receiving live metrics snapshots.
- Regression test that `/list` does not open or read `session.db`
  (inherits the release-branch test, must stay green).
- Regression test that `/info/{id}` for a **running** VM does not
  open `session.db`.
- Test that `/info/{id}` for a **stopped** VM successfully reads the
  one-shot session.db rollup.
- Timeout test: a slow snapshot cannot stall list/status indefinitely.
- `/metrics/json` contract test.
- `/metrics` low-cardinality text test.

### Gateway

- Status/metrics proxy tests preserving typed metric fields.
- Test that ask/enforcement split counters survive gateway translation.
- Test that MCP server summaries are not collapsed into one ambiguous
  total.
- Test that model provider/model/cost summaries and detection finding counters
  survive gateway translation without label/cardinality leaks.
- Test that forward-plugin health and counters survive gateway translation
  without endpoint/auth/error-string label leaks.

### Client / UI

- VM list/card renders live counters from typed JSON.
- VM status health renders model call count, provider/model summaries, token
  counts, estimated cost, and detection finding health from the live snapshot.
- Ask pass rate derives from `total_asks` and `asks_allowed` only.
- Enforcement/detection match stats render separately and do not collapse into
  a generic policy/rules total.
- Forward-plugin decision/observer health renders as a live VM status point,
  linked to local timeline/backtest/hunt evidence rather than duplicating it.
- Resource labels show configured / host-side / guest-side distinctly.

### Integration

- Boot a persistent VM with a pre-existing session.db full of prior
  totals; verify the accumulator immediately reports cumulative
  numbers without any caller-side opt-in.
- Boot an ephemeral VM; verify the accumulator starts at zero.
- Generate HTTP/DNS/MCP/file/model activity; verify live metrics
  update before shutdown.
- Stop the VM; verify the durable session DB still contains forensic
  truth and `/info/{stopped_id}` returns the one-shot rollup.
- Verify final list/status checks stay responsive under concurrent VM
  boot/teardown.

### Cardinality / adversarial

- Many unique URLs/paths/prompts/errors do not become exported labels.
- Many MCP tool names are bounded in exported metrics or kept JSON-only.
- Broken process IPC returns partial metrics/status instead of blocking
  all `/list` output.

## Open Questions Decided In This Doc

- **What is an ask?** The S06-pre Confirmer-resolved enforcement callback
  with matched `decision = "ask"`. Binary outcome. Four counters
  (total / allowed / denied / errored). No `asks_warned` in this family.
- **Does /info keep durable DB reads?** Not on running VMs. Running
  `/info` reads the accumulator only. The accumulator was seeded from
  the durable store at boot, so cumulative totals are already present.
  Stopped-VM `/info` falls back to a single session.db read; that is
  not a hot path.
- **First CPU/memory source?** Host process RSS/CPU now. Guest agent
  later. Hypervisor backend last (only if the abstraction exposes it
  cleanly -- see remaining open question).
- **MCP aggregation level?** Server-level in exported metrics.
  Tool-level top-N in JSON only.
- **Service vs gateway `/metrics` owner?** Service owns the canonical
  endpoint. Gateway proxies. UI consumes `/metrics/json` typed JSON.
- **Default exported labels?** Conservative: no path/URL/prompt/
  command/error string labels. High-cardinality detail in JSON only.

## Open Questions Remaining

- **How much guest visibility can we get from the hypervisor backends
  without a guest agent?** We have two backends behind the
  hypervisor abstraction:
  - macOS: Apple Virtualization.framework (`VZVirtualMachine` and
    friends). What does the framework actually expose for guest
    memory pressure / ballooning / per-vCPU utilization / virtio
    statistics? Anecdotally not much beyond configured size and
    process-level host RSS; needs concrete investigation before we
    decide whether the hypervisor layer is a useful source at all.
  - Linux: KVM via `kvm.h` and the surrounding `/proc/<pid>` /
    cgroup accounting. Likely richer (virtio-balloon, virtio-stats,
    `kvm_stat`, cgroup memory/cpu/io). Confirm what the abstraction
    surfaces.
  - Decision needed before phase B fully lands: which guest fields in
    `VmResourceMetrics` are sourced by which backend, and which
    require the guest agent. If hypervisor data is too thin in
    practice, document that and route all guest internals through
    the agent path. Do not invent numbers.
- **Schema migration when we add a counter mid-fleet.** A new counter
  introduced in a later release cannot retroactively reconstruct its
  cumulative history from `session.db` if the underlying events were
  not logged before the counter existed. Decide: do we backfill at
  seed time from raw event rows where possible, or accept that new
  counters start at zero per-VM the first time they are seen by a
  release that knows about them? Either is defensible; pick one and
  document it.
- **Aggregation across persistent VM "lives".** When a persistent VM
  is destroyed and recreated with the same name, does the new
  accumulator see the old durable history or start fresh? Tied to the
  ephemeral/persistent model invariants in CLAUDE.md.

## Possible Sub-Sprint Split

If this sprint feels too wide once execution starts (touching proto,
process, service, gateway, frontend, and durable-store policy in one
branch is genuinely a lot), split into:

- **S12a** -- proto contracts + process accumulator + boot seed (covers
  phases A and B above).
- **S12b** -- service `/list` and running-`/info` integration plus
  stopped-`/info` fallback (covers phase C minus the scrape endpoints).
- **S12c** -- service `/metrics/json` and `/metrics` scrape endpoints
  (the rest of phase C, isolated so the Prometheus/OTel format choices
  are reviewable on their own).
- **S12d** -- gateway proxy + UI typed-JSON consumption
  (phases D and E).
- **S12e** -- durable DB cleanup and final regression sweep
  (phase F).

The decision is deferred. We will evaluate the split when starting the
sprint, the same way S06-pre's sub-slices emerged during execution
rather than being committed up front.

## Coverage Ledger

- Unit/contract: proto serde roundtrips, accumulator update functions,
  `seed_accumulator_from_session_db` correctness, metric schema
  versioning.
- Functional: snapshot IPC, `/list` live-metrics path,
  `/info/{id}` running + stopped paths, `/metrics/json` shape,
  `/metrics` scrape shape.
- Adversarial: broken process IPC, slow snapshot, cardinality bombs,
  bad endpoint, retry exhaustion, crash-mid-batch loss bound.
- E2E/VM: persistent-VM-with-prior-totals seeding, ephemeral
  zero-start, boot-to-shutdown counter parity, durable DB still
  carries forensic truth post-stop.
- Telemetry: this sprint owns the telemetry redesign; no SQL on any
  hot fan-out path; VM status health exposes model/provider/cost,
  enforcement match, and detection finding counters from the live accumulator
  with boot-time recompute only.
- Performance: snapshot RTT bounded; seed cost paid once per VM
  launch; batched export overhead measured.

## Non-Goals (carried from the release handoff)

- Do not implement the full OTel exporter as part of the release-blocking
  hotfix; that work lives here.
- Do not add broad new metric names directly in service/gateway without
  proto contracts.
- Do not reintroduce SQLite reads into `/list` as a shortcut.
- Do not invent fake guest memory/CPU counters if the source is host
  process RSS/CPU.
- Do not collapse ask, HTTP, DNS, MCP, model, and filesystem denials
  into one ambiguous counter.
