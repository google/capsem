# Sprint Plan: Performance Observability Network Lab

## Why

We need to know whether speed work is needed, but current network numbers mix Capsem overhead with public internet latency, CDN behavior, DNS, and external service variability. That is not good enough for release discipline.

The answer is a local deterministic network lab plus proper debug-only tracing/OpenTelemetry spans. The goal is to measure the real Capsem path from guest request entering vsock through MITM, policy, security-event emission, DB write handoff, and response delivery.

## Key Decisions

- Use `tracing` spans and OpenTelemetry-compatible metrics as the timing mechanism.
- Keep the debug telemetry local-only: exported to an in-process/local collector, debug endpoint, JSON file, or benchmark artifact, but never to customer/upstream OTEL by default.
- Build a deterministic local upstream rather than benchmarking public websites for correctness/performance gates.
- Treat WebSockets as first-class. Existing WebSocket reject behavior is a product gap for the network lab.
- Treat model providers/endpoints as settings-defined data. The Rust extension point is protocol parsing and endpoint registry wiring, not per-provider enum expansion.
- Route model endpoint materialization through security events and action boundaries, not ad hoc MITM rewrites.
- Replace the old closed provider system. No second engine, no compatibility path that future code can keep using.
- Replace the old provider setup UI. Provider configuration is discovered from
  VM/user tool behavior or edited as settings records.
- Treat VM/workspace byte import and export as first-party security-event boundaries. Scanner plugins come after 1.3; this sprint builds the rail they require.
- Keep optimization separate from measurement. The first pass produces numbers; only then do we choose speed patches.

## T0: Contract

Files likely touched:

- `sprints/perf-observability-network-lab/*`
- `docs/draft/` or `docs/done/` only if the contract becomes durable.

Deliverables:

- Span naming contract.
- Local lab endpoint contract.
- Benchmark result schema.
- Test replacement list.
- Security constraints for debug telemetry.
- Provider-rule profile draft for OpenAI/Codex, Anthropic/Claude,
  Google/Gemini, and Ollama.

Span families:

- `capsem.launch.service`
- `capsem.launch.gateway`
- `capsem.launch.process`
- `capsem.launch.vm_boot`
- `capsem.launch.vsock_ready`
- `capsem.mitm.connection`
- `capsem.mitm.request`
- `capsem.mitm.policy.request`
- `capsem.mitm.security_actions`
- `capsem.mitm.model.request_policy`
- `capsem.mitm.upstream.prepare`
- `capsem.mitm.upstream.send`
- `capsem.mitm.policy.response`
- `capsem.mitm.model.response_policy`
- `capsem.mitm.body.chunk_hooks`
- `capsem.security_event.emit`
- `capsem.db.enqueue`
- `capsem.db.write_batch`
- `capsem.db.shutdown_flush`

Required labels:

- `protocol`: `http`, `https`, `websocket`, `mcp`, `dns`
- `event_type`: canonical runtime security event type where applicable
- `event_family`: canonical runtime family where applicable
- `decision`: `allow`, `deny`, `error`, `action`
- `provider`: model provider when applicable
- `rule_count`: number of candidate rules evaluated where cheap to know
- `body_kind`: `empty`, `tiny`, `1mb`, `10mb`, `gzip`, `sse`, `websocket`

Forbidden labels/fields:

- Raw authorization headers.
- Raw cookies.
- Raw OAuth tokens.
- Raw API keys.
- Raw request/response body content.

## T0.5: Provider Rule Profile

Before implementing the endpoint registry or compiler, create and review the
actual profile TOML we intend to ship.

Artifact:

- `sprints/perf-observability-network-lab/T0-provider-rules-draft.toml`

Providers:

- OpenAI / Codex CLI
- Anthropic / Claude Code
- Google AI / Gemini CLI
- Ollama

Rules:

- Rules are authored under the provider, such as `[ai.openai.rules.http_api]`.
- Detection and blocking use the same named rule and condition. Corp disables
  by overriding the same rule with `decision = "block"`, negative priority, and
  `corp_locked = true`.
- No provider rule should combine unrelated event families. Use one callback
  per rule.
- Credential rules are provider-owned too, such as
  `[ai.google.rules.gemini_api_key]`; they merge provider detection and broker
  capture.
- File rules name concrete tool-owned config paths. They must not copy rendered
  config content into settings.

Validation proof:

- Parse the draft TOML with the future provider-profile parser.
- Compile every provider rule to deterministic internal CEL/Sigma rules.
- Unit-test every rule against synthetic `http.request`, `dns.query`,
  `credential.observed`, `file.import`, and `model.request` events.
- Run local network/model fixtures and assert provider detections,
  credential-broker actions, model calls, and blocks appear in `session.db` and
  the DB-backed `/security/{id}/latest` ledger surface.
- Add corp override tests proving a copied provider rule with
  `decision = "block"` wins and cannot be undone by user discovery.
- Add priority validation proving negative priority is corp-only and
  `corp_locked = true` rules must use negative or zero priority.

## T1: Local Network Lab

Build a local deterministic mock server usable by tests and benchmarks.

Recommended implementation:

- Rust binary under `crates/` or a dev/test helper if the workspace pattern prefers it.
- Starts on `127.0.0.1:0` by default and prints JSON with bound ports.
- Supports plain HTTP first because host MITM upstream dialing can reliably reach `127.0.0.1:<port>` from the host side.
- Supports HTTPS/TLS if we can make the upstream trust story deterministic without relaxing production trust. If not, keep TLS measured on guest-side MITM plus local plain upstream and document the gap.
- Supports WebSocket echo and control endpoints.

Implementation:

- Workspace crate: `crates/capsem-mock-server`.
- Binary: `capsem-mock-server --addr 127.0.0.1:0`.
- Library helper: `spawn_mock_server()` with `addr()`, `base_url()`, and
  `shutdown()`.
- Ready output: one JSON object containing `service`, `http_addr`, `base_url`,
  and endpoint paths.
- HTTPS is intentionally not implemented in this upstream yet. The current
  deterministic lab is local plain HTTP plus WebSockets, while production MITM
  TLS behavior remains measured in the Capsem path.

Endpoints:

- `GET /tiny`: fixed small body.
- `GET /bytes/:size`: fixed deterministic bytes for `10kb`, `1mb`, `10mb`.
- `GET /gzip/:size`: gzip-encoded deterministic bytes.
- `GET /sse/model`: deterministic model-like SSE stream with tool-call-shaped events.
- `GET /slow-chunks`: deterministic delayed chunks.
- `GET /credential/response`: JSON body containing synthetic credential-looking material.
- `POST /echo`: echoes request metadata/body size, not raw secrets.
- `GET /deny-target`: path used by policy deny tests.
- `GET /ws/echo`: WebSocket echo.
- `GET /ws/ping`: WebSocket ping/pong behavior.
- `GET /ws/close`: deterministic close frame.

## T2: Debug OTEL Spans

Add spans around the production path, not a side ledger.

Debug config:

- `CAPSEM_DEBUG_TELEMETRY=local` enables local debug span filters only.
- OTLP exporter env vars such as `OTEL_EXPORTER_OTLP_ENDPOINT`,
  `OTEL_TRACES_EXPORTER`, and `OTEL_METRICS_EXPORTER` are ignored and reported
  as blocked unless `CAPSEM_ALLOW_UPSTREAM_OTEL=true` is explicitly set.
- This sprint still does not create an upstream exporter; the allowed flag is a
  future lab-only escape hatch, not a default path.

MITM/request path:

- vsock dispatch to MITM handler.
- protocol sniff and first read.
- guest TLS handshake.
- raw request head policy dispatch.
- security action materialization.
- model request policy body collect/parse/evaluate.
- upstream sender cache lock/ready.
- upstream TCP/TLS/hyper handshake.
- upstream request send.
- response head policy dispatch.
- model response policy body collect/parse/evaluate.
- chunk hook dispatch aggregate and per-hook histograms.
- telemetry build and security event enqueue.

Implementation status:

- MITM span-name constants live in
  `capsem_core::net::mitm_proxy::spans`.
- Request-path spans are wired with low-cardinality labels only. Raw domain and
  path fields were removed from the request span to match the T0 label
  contract.
- Security-event emit spans/metrics now live at the unified
  `emit_security_write` and `emit_security_write_blocking` handoff.
- DB writer spans and launch spans are separate T2.4-T2.5 work.

DB writer path:

- queue send wait time.
- batch size.
- batch SQL execution duration.
- shutdown flush/checkpoint duration.
- channel close/drop warnings.

Implementation status:

- DB writer spans/metrics live in `capsem-logger`, where the dedicated writer
  thread owns SQLite.
- Async, blocking, and try enqueue paths emit `capsem.db.enqueue` /
  `db.enqueue_wait_ms`.
- Writer-loop batches emit `capsem.db.write_batch`,
  `db.write_batch_total`, `db.write_batch_duration_ms`, and
  `db.write_batch_size`.
- Clean shutdown WAL checkpoint emits `capsem.db.shutdown_flush` /
  `db.shutdown_flush_ms`.

Launch path:

- service start readiness.
- gateway start readiness.
- process spawn.
- VM boot/provision.
- first control vsock handshake.
- first network request success.

Implementation status:

- Launch span names are defined in `capsem_core::telemetry`.
- Service startup/bind emits `capsem.launch.service`.
- Gateway spawn/ready emits `capsem.launch.gateway`.
- Service-side process spawn emits `capsem.launch.process_spawn` for provision
  and resume.
- Hypervisor boot emits `capsem.launch.vm_boot`.
- VM ready sentinel wait emits `capsem.launch.vsock_ready`.
- The first MITM request emits `capsem.launch.first_network_ready` once per
  process.

## T3: Benchmark Harness

Add repeatable benchmark modes:

- `capsem-bench mitm-local`
- optional host-side `uv run pytest tests/capsem-serial/test_mitm_local_benchmark.py -xvs`

Implementation status:

- `capsem-bench mitm-local` requires a local mock-server base URL from the
  first CLI argument or `CAPSEM_MOCK_SERVER_BASE_URL`. It does not run as
  part of `capsem-bench all`.
- The host-side artifact writer is gated by
  `CAPSEM_RUN_MITM_LOCAL_BENCH=1`, provisions a VM, runs the in-guest
  benchmark, pulls `/tmp/capsem-benchmark.json`, checks the synthetic API key
  is not stored in the benchmark JSON, and archives under
  `benchmarks/mitm-local/`.
- `capsem-logger` now has a Criterion `db_writer_pressure` benchmark over the
  real `DbWriter` and SQLite schema. Initial host numbers are 83K events/s at
  128-event bursts and roughly 150K events/s at 1024/4096-event bursts.
- The serial lifecycle benchmark now records min/mean/p50/p95/p99/max and the
  launch span contract in its JSON artifact.

Scenarios:

- tiny HTTP, concurrency 1/10/50.
- 1 MB HTTP, concurrency 1/10.
- gzip 1 MB, concurrency 1/10.
- SSE model stream, concurrency 1/10.
- denied request, concurrency 1/10/50.
- credential response capture, concurrency 1/10.
- WebSocket echo, 1/10/100 frames.
- DB event burst mixed HTTP/DNS/MCP/model/file/process if useful as a host-side logger benchmark.
- launch timing as host-side benchmark.

Metrics:

- p50/p95/p99 latency.
- requests/sec or frames/sec.
- bytes/sec.
- span breakdown percentiles.
- DB enqueue p50/p95/p99.
- DB batch write p50/p95/p99 and batch size.
- security events emitted by type/family.
- raw-secret leakage assertion.

## T4: Test Replacement

Replace default network correctness/performance tests that depend on public sites with local lab cases.

Implementation status:

- `capsem-bench http` and `capsem-bench throughput` no longer use public
  network targets by default. They prefer
  `CAPSEM_MOCK_SERVER_BASE_URL`, require
  `CAPSEM_BENCH_ALLOW_PUBLIC_NETWORK=1` for the old public targets, and
  otherwise emit structured skipped results.
- Guest diagnostics for plain HTTP proxying and proxy throughput now prefer the
  local mock-server URL and require
  `CAPSEM_RUN_PUBLIC_NETWORK_SMOKE=1` before running public Google/CDN probes.
- Public DNS/TLS/curl/provider diagnostics in `test_network.py`, public
  DNS/allowed-domain checks in `test_sandbox.py`, and the Google AI domain
  diagnostic in `test_ai_cli.py` now also require
  `CAPSEM_RUN_PUBLIC_NETWORK_SMOKE=1`.
- Positive public MCP `fetch_http`, `grep_http`, and `http_headers`
  diagnostics now also require `CAPSEM_RUN_PUBLIC_NETWORK_SMOKE=1`.
- WebSocket upgrades now tunnel through the MITM HTTP/1.1 upgrade path. A
  focused local test proves `101 Switching Protocols`, byte relay, and an
  allowed `/ws` session-DB event with status `101`.

Candidate replacements:

- `guest/artifacts/diagnostics/test_network.py` proxy throughput/public HTTP checks.
- `guest/artifacts/capsem_bench/http_bench.py` default target.
- `guest/artifacts/capsem_bench/throughput.py` default target.
- `guest/artifacts/capsem_bench/mitm_load.py` target model where applicable.
- Existing MITM tests that use intentionally nonexistent public domains for load characterization.

Keep as explicit smoke, not release gate:

- One public DNS smoke.
- One public HTTPS smoke.
- One real provider smoke only when credentials are configured and explicitly requested.

## T5: Hotspot Report

Produce `sprints/perf-observability-network-lab/hotspot-report.md` with:

- Local benchmark table.
- Span breakdown table.
- DB writer pressure table.
- Launch timing table.
- Comparison against existing public-network numbers.
- Recommendation: no speed sprint, targeted speed slice, or broader optimization sprint.

Implementation status:

- VM/MITM local benchmark JSON is archived at
  `benchmarks/mitm-local/data_1.0.1780763638_arm64.json`.
- The gated benchmark now fails if any HTTP/WebSocket scenario fails, queries
  the live session DB before teardown, and asserts expected paths, WebSocket
  `101` upgrade events, all `allowed` decisions, and no raw synthetic
  `capsem_test_` marker in audited net-event text columns.
- Lifecycle timing is archived at
  `benchmarks/lifecycle/data_1.0.1780763638.json` with total lifecycle
  p50/p95/p99 1057.0/1065.1/1065.8 ms.
- DB writer pressure is archived at
  `benchmarks/db-writer/data_1.0.1780763638_arm64.json` from Criterion output:
  128-event bursts p50/p95/p99 1.5188/1.5538/1.5588 ms, 1024-event bursts
  6.8931/7.0277/7.0382 ms, and 4096-event bursts
  27.0200/27.8743/28.0951 ms.
- `security.web.http_upstream_ports` is now a real settings-backed
  `int_list`, defaulting to `[80, 11434]`, so local benchmark policy can
  intentionally allow its dynamic mock-server port without weakening
  release defaults.
- Final hotspot report, litmus table, launch-number table, and optimization
  recommendation are complete. Live per-request metric export remains a future
  local OTEL/debug endpoint task and must not be routed through `/status`.

## T6: Boundary Foundations

This task reconciles the Ollama/custom-OpenAI litmus with the private-VT/PII
litmus. The common issue is not "which plugin do we add first"; it is whether
Capsem has one mandatory security-event rail for VM boundary operations.

Implementation status:

- Provider identity is now data from `[ai.<provider>]` settings/profile blocks.
- `ModelProtocol` is the typed Rust adapter boundary for wire parsing:
  Anthropic, OpenAI, Google, and native Ollama.
- `ProviderRuleProfile::endpoint_registry()` produces `ModelEndpointRegistry`
  records with provider id, display name, protocol, upstream URL, aliases,
  listen ports, credential setting slot, optional `credential:blake3:` ref,
  allowed remote targets, and tool-owned config files.
- Custom OpenAI-compatible endpoints can set `protocol = "openai-compatible"`
  and reuse the OpenAI adapter without a new Rust provider variant.
- Native Ollama has explicit request metadata parsing, usage extraction, LLM
  path gating, and a no-op SSE parser until a JSON-lines response adapter is
  wired.
- Runtime MITM routing now snapshots the live `ModelEndpointRegistry` from
  merged settings and passes the resulting typed protocol metadata through the
  request, credential broker, hook, and telemetry path.
- HTTP request materialization is owned by the security engine: action rules
  mutate the `SecurityEvent`, then
  `security_engine::materialize_http_request_for_upstream` creates the upstream
  copy. MITM no longer exposes a broker-substitution wrapper.
- `capsem-process` wires `model_endpoints` into `MitmProxyConfig` at launch and
  updates it during config reload with Policy V2 and security rules.
- The old MITM/SSE/interpreter `detect_ai_provider` domain matchers are gone.
  Hooks only trust `ConnMeta.ai_provider`; burn tests prove a known cloud domain
  without runtime metadata does not activate parsing.
- Native Ollama is proven through MITM using settings-owned endpoint data:
  `127.0.0.1:<port>` resolves to `ModelProtocol::Ollama` through host-plus-port
  registry matching, `/api/chat` is recognized as a model API path, native
  request/usage parsing populates `model_calls`, and the same event id anchors
  a canonical `model.call` security-rule ledger row.
- `file.import` and `file.export` are first-party security-event roots for
  VM/workspace boundary bytes. Service workspace upload logs import before
  writing, failed import logging fails closed, service download logs export
  before response, legacy `/write_file` logs import before guest write, and
  process-side `ReadFile` responses log export before resolving the read job.
- `fs_monitor` remains audit/reconciliation-only. It emits ordinary
  `file.event` rows for create/write/delete observations and may record
  matching rules, but it is not boundary enforcement proof and never emits
  `file.import` / `file.export`.
- The old frontend "setup needed because API keys are empty" signal is removed.
  Provider/broker status display must be built from discovery records, not from
  onboarding forms or direct key-validation UI.

This is a 1.3 foundation task. It does not implement dynamic plugin loading,
VirusTotal scanning, or PII scanning. It defines and proves the rails those
features must use after release.

### Model Endpoint Foundation

Problem:

- Current provider support is hardcoded around a closed `ProviderKind` enum.
- That cannot support "add Ollama by plugin only".
- It also cannot support a custom OpenAI-compatible endpoint without touching core.
- Endpoint routing is different from protocol parsing: `company-openai`,
  `local.openai`, and public OpenAI may all speak the OpenAI wire protocol but
  have different policy identities, credentials, aliases, and upstream URLs.

Target model:

```toml
[ai.ollama]
protocol = "ollama"
name = "Ollama"
url = "http://127.0.0.1:11434"
aliases = ["localhost", "127.0.0.1", "local.ollama"]
listen_ports = [11434]
allowed_remote_targets = ["127.0.0.1:11434", "local.ollama:11434"]

[ai.local_openai]
protocol = "openai-compatible"
name = "Local OpenAI"
url = "http://127.0.0.1:8080/v1"
aliases = ["local.openai"]
listen_ports = [11435]
allowed_remote_targets = ["127.0.0.1:8080"]

[ai.company_gateway]
protocol = "openai-compatible"
name = "Company Gateway"
url = "https://models.company.internal/v1"
aliases = ["models.company.internal"]
listen_ports = [443]
credential_setting_id = "ai.company_gateway.api_key"
credential_ref = "credential:blake3:<hash>"
allowed_remote_targets = ["models.company.internal:443"]
```

Rust boundary:

```rust
trait ModelProtocolAdapter {
    fn protocol(&self) -> &'static str;
    fn is_model_path(&self, path: &str) -> bool;
    fn parse_request(&self, body: &[u8]) -> RequestMeta;
    fn parse_response(&self, body: &[u8]) -> ModelResponseMeta;
    fn stream_parser(&self) -> Option<Box<dyn ProviderStreamParser + Send>>;
}
```

The trait shape is the future plugin seam, but the 1.3 implementation can use
built-in adapters registered in-process. The important invariant is that adding
an Ollama endpoint or custom OpenAI-compatible endpoint is settings data plus a
registered protocol adapter, not a new provider enum path.

Settings-owned endpoint fields:

- endpoint id.
- protocol parser id.
- provider/policy identity.
- guest aliases.
- guest listen/intercept ports.
- upstream URL.
- credential reference or named credential slot.
- allowed remote hosts/ports where needed.
- provider-owned rules under `[ai.<provider>.rules.<rule_id>]`, written beside
  the provider/profile object, not as top-level user-facing `policy.*`
  stanzas.
- compiled rule metadata: deterministic security-event rule ids, priority,
  action, optional detection level, plugin config, and corp-lock metadata.

Detection versus enforcement:

- Detection is rule metadata (`detection_level`), not an action. A provider
  rule may both report a detection level and enforce or run a plugin action.
- Enforcement rules are authored under the provider and compile to
  `allow`, `ask`, `block`, `preprocess`, or `postprocess` actions over
  first-party security events.
- Corporate config can lock provider/endpoint detection rules, disable a
  provider entirely, disable only specific endpoint aliases/ports/remote hosts,
  or inject higher-priority deny rules.
- Auto-discovery may propose or add user settings only when the corresponding
  corp policy permits it. It must not override a corp-disabled provider.
- Generated/default provider and credential rules come from the provider-owned
  profile config and compile directly into the security-event rule engine with
  deterministic ids and priority ordering. They do not generate old Policy V2
  callback rules.

Profile authoring shape is captured in
`sprints/perf-observability-network-lab/T0-provider-rules-draft.toml`. Do not
keep smaller toy examples in this plan; they drift too easily.

Provider-owned rules are intentionally one callback/event family per rule.
They must not rely on cross-family `OR` expressions. If a provider needs three
ways to be detected, it gets three small rules under the provider namespace.
Corp may author its own provider-owned rules the same way and mark them locked.

Compiled engine shape, internal only:

```text
ai.openai.rules.http_api -> profiles.rules.ai_openai_http_api
ai.openai.rules.config_credential_broker -> profiles.rules.ai_openai_config_credential_broker
ai.openai.rules.http_credential_broker -> profiles.rules.ai_openai_http_credential_broker
corp.rules.block_openai -> corp.rules.block_openai
```

Provider discovery and UI model:

- Built-in providers ship as default endpoint records.
- User/custom providers are created by settings edits or by security-path
  discovery when Capsem observes a credential, OAuth exchange, or known tool
  config inside the VM.
- Implemented discovery patch shape:

```toml
[ai.openai.discovery]
observed_at = "2026-06-06T10:00:00Z"
source = "http.header.authorization"
event_type = "http.request"
confidence = 1.0
credential_ref = "credential:blake3:<hash>"
trace_id = "trace-id"
```

- Credential brokerage writes the credential setting and discovery record
  atomically for built-in AI providers. Discovery-only user records merge
  against built-in endpoint/rule defaults; they do not duplicate tool config,
  endpoint defaults, or rules.
- Discovery records reject raw credentials and non-canonical runtime event
  types. Non-canonical observation names are not preserved into settings.
- The UI does not recreate the old setup wizard. `load_settings_response`
  exposes provider status and tool config source indexes, and the AI settings
  page shows detected/configured providers, broker refs, corp block state,
  endpoint routing, and discovered tool config sources from the
  settings/security-event index.
- Auto-detection may write a settings patch only through the broker/settings
  path, with a security event and substitution log proving the observation.

Tool-owned config files:

- The tool/user config file is the source of truth for tool-specific config.
  Examples: Codex `config.toml`, Claude JSON, Gemini JSON.
- Capsem settings must not store a second full copy of those files as provider
  settings. That is the DRY failure.
- Settings may store endpoint records, broker credential refs, and a discovery
  index for config sources: tool id, guest path, format, observed hash/version,
  owning endpoint id when inferred, and allowed Capsem overlays.
- Implemented source-index shape:

```toml
[tool_config_sources.codex_config]
tool_id = "codex"
guest_path = "/root/.codex/config.toml"
format = "toml"
observed_hash = "blake3:<64-hex>"
observed_version = "2026-06-06"
inferred_endpoint_ref = "ai.openai"
credential_refs = ["credential:blake3:<hash>"]
allowed_overlays = ["mcp_injection", "broker_placeholders"]
```

- The loader validates source-index records. Raw credential values, rendered
  `content`, malformed hashes, and malformed endpoint refs are rejected.
- Stored provider credentials must be broker refs. For AI/GitHub credential
  setting ids, the settings loader and batch-update API accept only empty
  values or `credential:blake3:` references; raw keys/tokens are rejected.
- Capsem-owned overlays are narrow generated patches such as MCP injection,
  telemetry/update disablement, or broker-reference placeholders. They are not
  a duplicated user config blob.
- Guest file materialization reads the current source config plus allowed
  overlays at the boundary, resolves only what is required, emits first-party
  file/security events, and does not write rendered config back into settings.
- Observed VM config files can update endpoint/credential discovery metadata
  when recognized; unrecognized files remain ordinary file events.

Required proof:

- Add an Ollama endpoint using only settings plus an `ollama` protocol adapter.
- Add a custom OpenAI-compatible endpoint using only settings and the existing
  `openai` protocol adapter.
- Both emit canonical `model.call` events with policy identity from settings.
- CEL rules can distinguish them by settings-owned provider/endpoint fields.
- Routing/materialization happens through the endpoint registry and security
  event/action path.
- A detected provider credential/config can create or update the corresponding
  settings record without an onboarding/setup wizard.
- The settings UI renders provider discovery and broker status from backend
  provider/tool-source records, not from setup state or raw provider key forms.
- Corp-disabled providers are not auto-added as usable endpoints; discovery may
  still emit an observation/security event explaining the blocked provider.
- Claude/Gemini/Codex config files remain tool-owned config sources. Capsem
  stores endpoint/credential/index metadata and overlays, not a second copy of
  their config content.
- Old hardcoded provider matching is gone, or only exists as private migration
  glue that delegates to the registry and is protected by burn guards.

Burn guard:

- No new direct matches on provider enum variants in MITM, policy, telemetry, or
  model parsing paths.
- Provider classification comes from endpoint registry results.
- Protocol parsing dispatch comes from protocol adapter registry results.
- Tests fail if new providers require editing a central closed enum.
- Tests prove corp-locked provider rules override user auto-discovery and user
  settings.
- Tests fail if provider setup UI or direct provider API-key settings become
  the only path to create a usable provider.
- Tests prove the settings response and frontend model carry provider status,
  broker refs, and tool config source indexes.
- Tests fail if config file materialization writes raw credential material or
  rendered config content into settings, or bypasses security-event emission.

### VM Byte Boundary Foundation

Problem:

- Private VT scanning of all imported files cannot be correct if some paths
  write bytes first and emit file events later.
- PII scanning of outbound data has the same mirror-image problem: export
  bytes/text must be first-party events before they cross the boundary.
- `fs_monitor` is useful reconciliation/audit, but it is post-write
  observation and cannot be the enforcement rail.

Target model:

```text
parser/materializer builds SecurityEvent
rules match SecurityEvent
actions may enrich/mutate SecurityEvent
enforcement decides
boundary operation happens only after allow
logger records event/action/decision
```

First-party boundary roots:

- `file.import`
- `file.export`
- `model.call`
- `http.request`
- `http.response`
- `dns.query`
- `dns.response`
- `mcp.tool_call`
- `process.exec`

Implementation status:

- `file.import` and `file.export` are closed `RuntimeSecurityEventType` values,
  not only CEL field roots.
- `FileAction::Imported` maps to `file.import`; `FileAction::Exported` maps to
  `file.export`; create/write/read/delete/restored file observations remain
  `file.event`.
- New `security_rule_events` and `security_ask_events` tables have strict
  event-type CHECK constraints, including `file.import` and `file.export`, and
  rejecting stale names such as `file.ingress`, `model.request`, and
  `dns.response`.

Required import proof:

- Service file upload does not write directly into the workspace without a
  `file.import` decision.
- API/CLI `write_file` does not send bytes to the guest without a
  `file.import` decision.
- Restore/snapshot paths do not mutate workspace files without a
  `file.import` decision.
- Network/MITM file-like bodies have a documented pre-delivery event path when
  they are materialized as files.
- `fs_monitor` remains audit/reconciliation and cannot be the only proof of
  enforcement.

Required export proof:

- File download/export emits `file.export` before bytes leave the
  VM/workspace boundary.
- HTTP/model/MCP outbound bodies that can contain PII are represented as
  first-party events before delivery.
- Future PII scanning can attach as an action over the event without inventing
  a second export engine.

Future scanner action litmus, deferred until after 1.3:

```text
file.import
  -> rule match
  -> action: scanner.virustotal_private.scan
  -> enriched event with scan verdict
  -> enforcement decision
  -> materialize file

file.export
  -> rule match
  -> action: scanner.pii.scan
  -> enriched event with PII verdict
  -> enforcement decision
  -> release bytes
```

Burn guard:

- No production code path may write/read boundary bytes directly and then rely
  on a later monitor event as enforcement proof.
- New boundary operations must call the security-event materialization gate.
- Tests fail on direct service/process/MCP file writes that bypass the gate.

## Done Means

- Local lab is deterministic and test-owned.
- Debug OTEL spans exist on the production path but are local/debug-only.
- WebSocket behavior is tested through the lab.
- Network tests no longer depend on public websites by default.
- DB write and launch timing are visible.
- The litmus test table in `MASTER.md` is filled with numbers.
- The boundary foundation litmus proves model endpoints are data and file
  import/export are mandatory first-party security events, not side monitors.
- Provider UI/setup burn is implemented and documented: provider discovery and
  config source indexing are settings/security-event flows, not onboarding
  state or duplicated config storage.

## Coverage Matrix

| Category | Required Proof |
| --- | --- |
| Unit/contract | Span name/label contract tests; debug telemetry redaction tests; local lab endpoint tests. |
| Functional | HTTP/gzip/SSE/WebSocket through Capsem to local lab. |
| Adversarial | denied request, slow chunks, disconnects, malformed gzip, WebSocket close/error paths, synthetic credential leak attempts. |
| E2E/VM | Fresh VM runs `capsem-bench mitm-local` and stores JSON. |
| Telemetry/observability | Spans and metrics present locally; no upstream exporter by default; no raw secret fields. |
| Performance | p50/p95/p99 for local scenarios; DB enqueue/write; launch timing. |
| Boundary architecture | Ollama and custom OpenAI-compatible endpoints are settings/discovery-defined without core provider enum expansion; tool config files remain source-of-truth config sources with narrow overlays/index metadata; file import/export roots are mandatory rails for future VT/PII actions. |
