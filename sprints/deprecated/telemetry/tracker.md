# Sprint: Telemetry & Observability

## Design Doc
- [ ] `docs/design/telemetry-observability.md` -- full design doc covering format spec, architecture, metrics taxonomy, config, buffering
- [ ] Metrics taxonomy: all counters, gauges, histograms with labels and bucket configs
- [ ] Architecture diagram: capsem-process -> capsem-service -> capsem-telemetry -> OTLP
- [ ] MITM proxy timing breakdown: request lifecycle phases (tls_handshake, upstream_connect, ttfb, etc.)
- [ ] Capsem Telemetry Format v1 spec: naming conventions, label conventions, versioning rules
- [ ] OTLP mapping: resource attributes, metric attributes, severity mapping
- [ ] Corporate config: 3 settings (enabled, endpoint, auth_token), lockdown scenarios
- [ ] Buffering strategy: in-memory + on-disk WAL, failure/recovery behavior
- [ ] Service-level event gap analysis: current state vs proposed for each lifecycle event
- [ ] Implementation roadmap with dependency ordering
- [ ] Commit: `docs: telemetry observability design doc`

## Phase 1: Foundation -- Types, Config, Telemetry Crate

### Telemetry core types (`capsem-core/src/telemetry/`)
- [ ] `mod.rs` -- module root, re-exports
- [ ] `metrics.rs` -- `VmMetrics` (per-VM handle), `ServiceMetrics` (service-level), `MetricPoint` enum (Counter/Gauge/Histogram)
- [ ] `config.rs` -- `TelemetryConfig` struct, `settings_to_telemetry_config()` builder

### IPC protocol (`capsem-proto/src/ipc.rs`)
- [ ] `ToTelemetry` enum: Ping, MetricsBatch, Shutdown
- [ ] `FromTelemetry` enum: Pong, Status
- [ ] `MetricsSource` enum: Service | Process { vm_id, session_id, vm_name }
- [ ] `MetricsBatch` struct: source + Vec<MetricPoint>

### Config (`config/defaults.toml`)
- [ ] `[settings.telemetry.enabled]` -- bool, default false
- [ ] `[settings.telemetry.endpoint]` -- url, default ""
- [ ] `[settings.telemetry.auth_token]` -- apikey, default ""
- [ ] Wire into `MergedPolicies` via `policy_config/builder.rs`

### Telemetry binary (`crates/capsem-telemetry/`)
- [ ] New crate: `Cargo.toml`, `src/main.rs`
- [ ] UDS listener on `~/.capsem/run/telemetry.sock`
- [ ] Receive `ToTelemetry` messages, respond `FromTelemetry`
- [ ] Skeleton: accept MetricsBatch, log receipt, no export yet

### Dependencies
- [ ] Add `opentelemetry`, `opentelemetry-otlp`, `opentelemetry_sdk` to workspace
- [ ] Wire deps into `capsem-core` and `capsem-telemetry`

### Tests
- [ ] Unit: TelemetryConfig from resolved settings (defaults, user override, corp lock)
- [ ] Unit: MetricPoint serde roundtrip
- [ ] Unit: VmMetrics counter/gauge/histogram recording
- [ ] Commit: `feat: telemetry foundation -- types, config, capsem-telemetry crate`

## Phase 2: Telemetry Process -- Aggregation, OTLP Export, Buffering

### MetricsAggregator (`capsem-telemetry`)
- [ ] Aggregate incoming MetricsBatch into OpenTelemetry SDK meter provider
- [ ] Per-VM label scoping (vm_id on all metrics)
- [ ] Periodic export interval (10s)

### OTLP Exporter
- [ ] Configure `opentelemetry-otlp` exporter from TelemetryConfig (endpoint, auth_token)
- [ ] Protocol auto-detection or config (gRPC vs HTTP)
- [ ] Resource attributes: `service.name=capsem`, `service.version`

### On-disk buffer (`~/.capsem/telemetry/buffer/`)
- [ ] On export failure: serialize batch to JSONL file
- [ ] On startup: drain existing buffer FIFO
- [ ] Periodic retry (30s)
- [ ] FIFO eviction at 50MB cap
- [ ] Crash-safe: one JSON object per line, skip unparseable on recovery

### Service spawns telemetry process
- [ ] capsem-service: spawn `capsem-telemetry` on startup if `telemetry.enabled`
- [ ] Pass args: `--uds-path`, `--endpoint`, `--auth-token`
- [ ] Async reaper task
- [ ] Periodic health check (Ping/Pong), restart on crash
- [ ] Graceful shutdown: send Shutdown, wait for flush

### Tests
- [ ] Unit: aggregator merges batches from multiple sources
- [ ] Unit: buffer write/read/eviction
- [ ] Integration: start telemetry process, send batches, verify OTLP export (mock collector)
- [ ] Commit: `feat: capsem-telemetry process -- OTLP export + disk buffer`

## Phase 3: Instrument Per-VM Code

### MITM proxy timing breakdown (`capsem-core/src/net/mitm_proxy.rs`)
- [ ] Add timing fields to `NetEvent`: `tls_handshake_ms`, `upstream_connect_ms`, `upstream_tls_ms`, `ttfb_ms`
- [ ] Instrument TLS accept (~L214)
- [ ] Instrument upstream TCP connect (~L446)
- [ ] Instrument upstream TLS (~L461)
- [ ] Instrument time to first response byte
- [ ] Record `capsem_request_duration_seconds` histogram
- [ ] Record `capsem_request_ttfb_seconds` histogram
- [ ] Record `capsem_tls_handshake_seconds` histogram
- [ ] Record `capsem_upstream_connect_seconds` histogram
- [ ] Record `capsem_requests_total` counter (with domain, decision, method, status labels)
- [ ] Record `capsem_request_bytes_total` counter (sent/received)
- [ ] Record `capsem_policy_denials_total` counter
- [ ] Record `capsem_inflight_requests` gauge (inc/dec)
- [ ] Schema migration: add timing columns to `net_events` table

### AI traffic metrics (`capsem-core/src/net/mitm_proxy.rs`)
- [ ] Record `capsem_model_calls_total` counter
- [ ] Record `capsem_tokens_total` counter (input/output)
- [ ] Record `capsem_estimated_cost_total` counter
- [ ] Record `capsem_tool_calls_total` counter
- [ ] Record `capsem_model_call_duration_seconds` histogram

### MCP gateway (`capsem-core/src/mcp/gateway.rs`)
- [ ] Record `capsem_mcp_calls_total` counter
- [ ] Record `capsem_mcp_call_duration_seconds` histogram

### FS monitor (`capsem-core/src/fs_monitor.rs`)
- [ ] Record `capsem_file_events_total` counter (by action)

### Snapshot scheduler (`capsem-core/src/auto_snapshot.rs`)
- [ ] Add `duration_ms` to `SnapshotEvent`
- [ ] Record `capsem_snapshots_total` counter
- [ ] Record `capsem_snapshot_duration_seconds` histogram
- [ ] Schema migration: add `duration_ms` to `snapshot_events` table

### Boot timing (`capsem-core/src/host_state.rs`)
- [ ] Record `capsem_boot_duration_seconds` histogram per stage

### IPC forwarding
- [ ] capsem-process: batch VmMetrics every 5s into `TelemetryBatch`
- [ ] Send via existing ProcessToService IPC (add `TelemetryBatch` variant)
- [ ] capsem-service: relay received batches to capsem-telemetry UDS

### Tests
- [ ] Unit: timing fields populated in NetEvent
- [ ] Unit: histogram/counter updates correct for each instrumentation point
- [ ] Integration: boot VM, make requests, verify metrics arrive at telemetry process
- [ ] Commit: `feat: per-VM metrics instrumentation -- proxy timing, AI, MCP, FS, snapshots`

## Phase 4: Service-Level Metrics + Events

### ServiceMetrics in capsem-service
- [ ] `capsem_active_vms` gauge (inc on create, dec on stop/crash)
- [ ] `capsem_vm_uptime_seconds` gauge per VM
- [ ] `capsem_service_uptime_seconds` gauge
- [ ] `capsem_vm_boots_total` counter
- [ ] `capsem_vm_crashes_total` counter
- [ ] `capsem_image_ops_total` counter (create/delete)

### Lifecycle event emission
- [ ] VmCreated: on `provision_sandbox()` success
- [ ] VmBooted: when capsem-process reports ready
- [ ] VmStopped: on `shutdown_vm_process()`
- [ ] VmCrashed: on dead PID detection in `cleanup_stale_instances()`
- [ ] VmDeleted: on `handle_delete()`
- [ ] ImageCreated: on `handle_fork()` success
- [ ] ImageDeleted: on `handle_image_delete()` success

### Service writes to main.db
- [ ] Add `SessionIndex` to service state
- [ ] Create session record on VM provision
- [ ] Update status on stop/crash
- [ ] Record image operations

### Background health monitor
- [ ] Periodic PID check (every 30s) instead of lazy on-API-request only
- [ ] Detect crashed VMs proactively

### `/health` endpoint
- [ ] Returns: service uptime, active VM count, telemetry process status

### Tests
- [ ] Unit: ServiceMetrics gauge/counter updates
- [ ] Integration: create/stop VM, verify lifecycle metrics + main.db records
- [ ] Commit: `feat: service-level metrics, lifecycle events, /health endpoint`

## Phase 5: Format Documentation
- [ ] `site/src/content/docs/reference/telemetry-format.mdx` -- public spec
- [ ] Full metrics reference table (name, type, labels, buckets)
- [ ] OTLP attribute mapping
- [ ] JSON examples for each metric type
- [ ] Versioning rules
- [ ] Corporate deployment guide (endpoint, auth, lockdown)
- [ ] Skills update: new `dev-telemetry` skill
- [ ] CHANGELOG entries
- [ ] Commit: `docs: Capsem Telemetry Format v1 spec + dev-telemetry skill`

## Notes
- Per-VM session.db unchanged -- metrics are additive, not a replacement
- capsem-telemetry is security-isolated: cross-VM visibility + OTLP egress only
- OTLP deps always included (no feature gating)
- Config is 3 settings only: enabled, endpoint, auth_token
