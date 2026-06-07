# OpenTelemetry Metrics Handoff

Last updated: 2026-05-15

## Why This Exists

During release-debug-loop final verification, the release branch exposed a status-path coupling we do not want in production: service `/list` was decorating every running VM by opening each VM's `session.db` and querying aggregate telemetry. That made the list/status path depend on per-session SQLite at exactly the wrong time: while VM boot, teardown, WAL checkpointing, gateway polling, and install verification can all be happening under load.

The short-term release fix is intentionally narrow:

- `/list` no longer reads `session.db`.
- `/list` keeps only in-memory service state plus a placeholder hook for future live metrics.
- Single-VM/detail paths may still read durable session DB telemetry until the OTel sprint replaces that with live snapshots where appropriate.
- SQLite remains the durable forensic/audit store, not the hot status source.

This document is the handoff for the real metrics/OTel sprint so the next agent can build the correct architecture instead of reintroducing SQL scans into `/list`.

## Current Patch On This Branch

Code behavior after the release hot-path patch:

- `crates/capsem-service/src/main.rs` has `enrich_telemetry_from_session_db(...)` for durable DB-backed enrichment.
- `handle_info` still calls `enrich_telemetry_from_session_db(...)` for a single running VM detail response.
- `handle_list` calls `attach_list_live_metrics_placeholder(...)` instead of opening `session.db`.
- The placeholder is intentionally a no-op with a `FIXME(otel-sprint)` note.
- A regression test creates a valid empty `session.db` for a running VM and proves `/list` leaves SQLite-backed counters unset.

Important constraint: do not make `/list` read SQLite again. If live counters are needed in list/status, source them from capsem-process through typed IPC snapshots.

## Current Telemetry Source

Today, durable session telemetry is stored in `<session_dir>/session.db` through `capsem_logger::DbWriter` and read through `capsem_logger::DbReader`.

The old `/list` enrichment used these reader calls:

- `session_stats()` for tokens, estimated cost, tool calls, HTTP/network totals, allowed/denied counts, and model call count.
- `file_event_count()` for one coarse file-event total.
- `mcp_call_stats()` for one coarse MCP call total.

That is acceptable for forensic/detail queries, support bundles, stats pages, or post-session rollups. It is not acceptable for live list/status fan-out.

## IPC Reality Check

There are two relevant wire planes. Keep them separate.

Service to capsem-process:

- Socket: per-VM Unix domain socket.
- Rust types: `capsem_proto::ipc::{ServiceToProcess, ProcessToService}`.
- Transport: `tokio-unix-ipc`.
- Serialization: `bincode`.
- Existing actions include exec, MCP call tool, file read/write/delete, shutdown, suspend, and related responses.

Host/process to guest control bridge:

- Rust types: `capsem_proto::{HostToGuest, GuestToHost}`.
- Serialization: `rmp-serde` MessagePack.
- Framing: 4-byte big-endian payload length followed by MessagePack payload.
- Existing actions include boot config, env, exec, file read/write/delete, shutdown, suspend, ack/replay, and guest status messages.

Other MessagePack side channels exist too, for example DNS/audit/MCP aggregator framing. That does not mean service/process IPC is MessagePack. For live VM metrics, the natural path is a new typed service/process IPC request/response in `capsem-proto::ipc`, encoded by bincode like the rest of that channel.

## Target Architecture

The shape we want:

- `capsem-process` owns live VM metrics accumulation for the VM it supervises.
- `capsem-proto` owns shared metrics contracts and metric naming primitives so service, process, gateway, CLI, frontend, and tests do not drift.
- `capsem-service` asks each process for a `VmMetricsSnapshot` over typed IPC when it needs live status metrics.
- `capsem-service` exposes typed JSON for product/UI consumers and Prometheus/OpenTelemetry-compatible counters for scrape/export consumers.
- `capsem-gateway` exposes or proxies the service metrics surfaces so client/UI and external observers do not need private service sockets.
- Client/UI consume typed JSON, not Prometheus text.
- `session.db` remains durable truth for audit/history and post-session forensic summaries.

A rough data path:

```text
VM runtime events
  -> capsem-process live accumulator
  -> ServiceToProcess::GetMetricsSnapshot / ProcessToService::MetricsSnapshot
  -> capsem-service /list, /info, /metrics/json, /metrics
  -> capsem-gateway status/metrics routes
  -> CLI, tray, frontend, external OTel/Prometheus collectors
```

## Suggested Proto Module

Add a shared module in `crates/capsem-proto`, for example `capsem_proto::metrics`.

Sketch, not final API:

```rust
pub struct VmMetricsSnapshot {
    pub schema_version: u32,
    pub vm_id: String,
    pub persistent: bool,
    pub lifecycle: VmLifecycleMetrics,
    pub resources: VmResourceMetrics,
    pub ask: VmAskMetrics,
    pub http: VmHttpMetrics,
    pub dns: VmDnsMetrics,
    pub model: VmModelMetrics,
    pub mcp: VmMcpMetrics,
    pub filesystem: VmFilesystemMetrics,
    pub captured_at_unix_ms: u64,
}

pub struct VmAskMetrics {
    pub total_asks: u64,
    pub asks_allowed: u64,
    pub asks_warned: u64,
    pub asks_denied: u64,
    pub asks_errored: u64,
}
```

Principles:

- Put contracts in proto first.
- Keep counters monotonic where possible.
- Avoid high-cardinality labels in exported metric names.
- Keep rich/high-cardinality details in JSON only, bounded and summarized.
- Derive pass rates from counters rather than storing ratios.

## Metric Taxonomy

### Ask/Policy

We should not have one vague `policy_denials` counter. The ask flow needs pass-rate math.

Minimum counters:

- `total_asks`
- `asks_allowed`
- `asks_warned`
- `asks_denied`
- `asks_errored`

Derived:

- ask pass rate = `(asks_allowed + asks_warned) / total_asks`
- ask denial rate = `asks_denied / total_asks`
- ask error rate = `asks_errored / total_asks`

Open question: define exactly which event planes are an "ask". Candidate sources include policy hooks, model mediation, tool approvals, and MCP tool invocation decisions. Do not blend unlike decisions without naming them.

### HTTP/HTTPS

Avoid `net_` for the user-facing metric family unless the metric really spans all network classes. The MITM/user-visible path is closer to HTTP/HTTPS.

Suggested counters:

- `http_requests_total`
- `http_requests_allowed_total`
- `http_requests_warned_total`
- `http_requests_denied_total`
- `http_requests_errored_total`
- `http_bytes_sent_total`
- `http_bytes_received_total`

Optional JSON-only summaries:

- top blocked domains
- top upstream status classes
- recent denial reasons

Avoid path, URL, prompt, command, and raw error labels in Prometheus/OTel metrics.

### DNS

DNS should be separate from HTTP.

Suggested counters:

- `dns_queries_total`
- `dns_queries_allowed_total`
- `dns_queries_warned_total`
- `dns_queries_denied_total`
- `dns_queries_rewritten_total`
- `dns_queries_errored_total`

### MCP

`tool_calls` and `mcp_calls` are easy to confuse. Split them by plane.

Suggested MCP counters:

- `mcp_tool_invocations_total`
- `mcp_tool_invocations_allowed_total`
- `mcp_tool_invocations_warned_total`
- `mcp_tool_invocations_denied_total`
- `mcp_tool_invocations_errored_total`
- `mcp_servers_connected_total`
- `mcp_servers_disconnected_total`
- `mcp_server_errors_total`

Aggregation:

- Aggregate by MCP server name where cardinality is bounded by configured servers.
- Consider top-N tool summaries in JSON only.
- Be cautious with tool-name labels in exported metrics because remote/user-defined tools can explode cardinality.

### Filesystem

`file_events` is too coarse.

Suggested counters:

- `fs_reads_total`
- `fs_writes_total`
- `fs_creates_total`
- `fs_deletes_total`
- `fs_restores_total`
- `fs_errors_total`
- `fs_bytes_read_total`
- `fs_bytes_written_total`

If event source can distinguish workspace/system/session files, keep that as a low-cardinality enum label or JSON field. Do not use raw paths as exported labels.

### Model

Suggested counters:

- `model_requests_total`
- `model_requests_allowed_total`
- `model_requests_warned_total`
- `model_requests_denied_total`
- `model_requests_errored_total`
- `model_input_tokens_total`
- `model_output_tokens_total`
- `model_estimated_cost_micros_total`

Use integer micros for cost in counters. Floating-point cost belongs in rendered summaries, not core counter storage.

### Resources

We probably need these for a serious VM status surface, but source availability differs by platform.

Available now or likely easy:

- configured RAM MB
- configured vCPU count
- host process PID
- host process RSS bytes from OS process inspection
- host process CPU time/percentage from OS process inspection
- session/workspace/rootfs-overlay byte usage from filesystem metadata

Potential later sources:

- guest-reported memory pressure/used/free via guest agent
- guest-reported disk IO and network IO
- hypervisor-specific counters if Apple Virtualization.framework/Linux backend exposes them through our abstraction

Important: if the hypervisor API does not expose guest memory usage directly, do not fake it. Use host process RSS as an explicitly named host-side resource metric and add guest metrics later.

### Lifecycle

Suggested fields/counters:

- current lifecycle state
- uptime seconds
- boot count
- restart count
- suspend count
- resume count
- shutdown count
- unexpected exit count
- last transition timestamp
- last error for JSON only

Do not export raw last-error strings as labels.

## Service And Gateway Surfaces

Recommended surfaces:

- Service `/list`: hot, bounded, no SQLite. Can include live `VmMetricsSnapshot` summaries once IPC exists.
- Service `/info/{id}`: can include live snapshot and may include durable DB detail while the migration is in progress.
- Service `/metrics/json`: typed aggregate JSON for gateway/client/UI/tests.
- Service `/metrics`: Prometheus text/OpenTelemetry scrape-friendly metrics.
- Gateway `/status`: keep existing product status shape and add/forward typed metrics summaries where UI needs them.
- Gateway `/metrics/json`: proxy service typed metrics JSON.
- Gateway `/metrics`: proxy or render scrape metrics if gateway is the public observation boundary.

The client/UI should prefer typed JSON. Prometheus/OpenTelemetry text is for collectors.

## Implementation Plan

1. Proto contracts

- Add `capsem_proto::metrics` structs.
- Add `ServiceToProcess::GetMetricsSnapshot` and `ProcessToService::MetricsSnapshot` or similarly named variants.
- Add bincode roundtrip tests.
- Add schema/version compatibility tests.

2. Process accumulator

- Add a per-process in-memory accumulator.
- Update it at the same points that currently write durable telemetry.
- Keep accumulator update functions testable without SQLite.
- Do not block VM control flow on metrics export.

3. Service integration

- Add a bounded-time metrics snapshot query helper.
- Use it from `/list` without holding `instances` lock across IPC.
- Use it from `/info` for live counters.
- Add `/metrics/json` typed response.
- Add `/metrics` scrape response with low-cardinality labels.

4. Gateway integration

- Preserve/proxy service typed metrics JSON.
- Add gateway tests that ensure client/UI metrics survive gateway translation.
- Decide whether gateway renders Prometheus text itself or simply proxies service `/metrics`.

5. Client/UI integration

- Consume typed metrics JSON for VM cards/status panels.
- Avoid parsing Prometheus text in frontend code.
- Render ask pass rate from counters.
- Render resource metrics with clear source labels, for example "host process RSS" vs "guest memory used".

6. Durable DB cleanup

- Keep `session.db` for audit/history.
- Remove any remaining SQL reads from hot status/list paths.
- Make detail/history views explicit about whether they show live snapshot, durable summary, or both.

## Testing Plan

Proto:

- Bincode roundtrip for new service/process IPC variants.
- Serialization compatibility for `VmMetricsSnapshot`.
- Metric schema version test.

Process:

- Unit tests for ask counters and pass-rate inputs.
- Unit tests for HTTP/DNS/model/MCP/filesystem/resource accumulator updates.
- Tests that accumulator functions do not require a `DbWriter` or SQLite.

Service:

- Mock IPC tests for `/list` receiving live metrics snapshots.
- Regression test that `/list` does not open/read `session.db`.
- Timeout test: a slow metrics snapshot cannot stall list/status indefinitely.
- `/metrics/json` contract test.
- `/metrics` low-cardinality text test.

Gateway:

- Status/metrics proxy tests preserving typed metric fields.
- Test that gateway does not drop ask split counters.
- Test that gateway does not collapse MCP server summaries into one ambiguous total.

Client/UI:

- VM list/card renders live counters from typed JSON.
- Ask pass rate is derived from `total_asks`, `asks_allowed`, and `asks_warned`.
- Resource labels distinguish configured, host-side, and guest-side sources.

Integration:

- Boot a VM, generate HTTP/DNS/MCP/file/model activity, and verify live metrics update before shutdown.
- Stop VM, verify durable session DB still contains forensic truth.
- Verify final list/status checks stay responsive under concurrent VM boot/teardown.

Cardinality/adversarial:

- Many unique URLs/paths/prompts/errors do not become metric labels.
- Many MCP tool names are bounded in exported metrics or kept JSON-only/top-N.
- Broken process IPC returns partial metrics/status rather than blocking all `/list` output.

## Open Questions For The OTel Sprint

- What exactly is an "ask"? Decide the event boundary before wiring counters.
- Should `/info` keep durable DB enrichment after live snapshots exist, or split live detail from history detail?
- Which source should we use first for CPU/memory: host process inspection, guest agent, hypervisor API, or a staged combination?
- How much MCP aggregation by server is acceptable in exported metrics? Server-level probably yes; tool-level should likely be JSON-only/top-N unless bounded.
- Should gateway own public `/metrics`, or should service be the scrape endpoint and gateway only provide UI JSON?
- What labels are acceptable for customer/self-hosted deployments? Default should be conservative.

## Non-Goals For The Release Branch

- Do not implement the full OTel exporter here.
- Do not add broad new metric names directly in service/gateway without proto contracts.
- Do not reintroduce SQLite reads into `/list` as a shortcut.
- Do not invent fake guest memory/CPU counters if the source is host process RSS/CPU.
- Do not collapse ask, HTTP, DNS, MCP, model, and filesystem denials into one ambiguous counter.
