# Meta Sprint: perf-observability-network-lab

## Status

| Sprint | Status | Purpose |
| --- | --- | --- |
| T0: Contract | Done | Debug-only span/label contract and current network-test replacement inventory are frozen. |
| T0.5: Provider Rule Profile | Done | Built-in provider defaults plus user/corp overlays compile into runtime security-event rules and settings response; CEL fixtures, MITM/session DB provider-rule proof, and the `/security/{id}/latest` ledger surface are covered. |
| T1: Local Network Lab | Done | Added deterministic local HTTP/WebSocket/SSE/gzip upstream and test lifecycle helper; deterministic HTTPS remains documented as a gap unless production-safe trust plumbing is added later. |
| T2: Debug OTEL Spans | Done | Local-only debug telemetry policy, MITM request-path spans, security-event emit spans/metrics, DB writer spans/metrics, and launch spans are implemented. |
| T3: Benchmark Harness | Done | Added `capsem-bench mitm-local`, a gated host VM artifact writer, DB writer pressure benchmark, and lifecycle percentile artifact support. |
| T4: Test Replacement | Done | Public HTTP/throughput defaults prefer local lab or skip; guest diagnostic public probes are explicit smoke-only; WebSocket-through-MITM is locally proven. |
| T5: Hotspot Report | Done | VM/MITM local matrix, session-DB proof, DB writer pressure, lifecycle timing, and hotspot recommendation are archived. |
| T6: Boundary Foundations | Done | Model provider identity is split from typed protocol adapters; endpoint schema covers aliases, listen ports, credential refs, and allowed targets; MITM runtime endpoint routing now uses the live registry; native Ollama is proven through canonical `model.call`; request materialization flows through security-engine actions; `file.import`/`file.export` are first-class event types; service upload/download, legacy write/read, fs-monitor audit-only boundaries, provider discovery settings patches, tool config source indexes, provider-owned security-event rules, and provider/broker settings UI are proven. |

## Non-Negotiables

- Use `tracing` spans and OpenTelemetry-compatible metrics. Do not invent a parallel timing ledger.
- Debug OTEL is local-only and benchmark/debug-only. It must not export to upstream/customer OTEL by default.
- Benchmarks must use deterministic local endpoints, not public websites, for regression gates.
- WebSockets are first-class in the local lab and in the MITM path. A reject-only WebSocket posture is no longer sufficient for this sprint.
- Model providers/endpoints must be settings-defined data. Rust should provide protocol parser registries/boundaries, not require a new enum variant for every endpoint.
- Provider and credential detection live as rules over first-party security
  events, with corp-lock support. No separate provider matcher schema.
- VM boundary materialization must flow through security-event/action plumbing, not MITM, service, fs-monitor, or helper side rewrites.
- Burn the old provider wheel. Do not leave a parallel closed-enum provider system beside the endpoint/protocol registry.
- Burn the old provider setup UI. Provider identity comes from defaults,
  settings edits, or VM/security-path discovery, not install/onboarding forms.
- No bytes enter or leave the VM/workspace boundary without becoming a first-party security event before the boundary operation when enforcement is possible.
- DB write latency, queue wait, batch size, and flush/shutdown time must be measured.
- Launch timing must cover service, gateway, process spawn, VM boot/provision, first vsock/control readiness, and first usable network request.
- Security invariants stay above performance. Spans must preserve reference-only credential logging and never record raw secrets.

## Scope

This sprint is not "make everything fast" by guessing. It builds a deterministic lab and proper observability so Capsem can see where time is actually spent:

- HTTP request through vsock/MITM to local upstream and back.
- HTTPS/TLS termination and upstream dial behavior where deterministic TLS is possible.
- WebSocket upgrade, frame relay, close, ping/pong, and policy/telemetry behavior.
- Gzip response decompression.
- SSE/model-like streaming and parser hook overhead.
- Credential-broker capture/redaction path with synthetic local secrets.
- Ollama, local OpenAI-compatible, and remote OpenAI-compatible endpoint routing as litmus cases for settings-defined model endpoints.
- Provider-rule profile TOML for OpenAI/Codex, Anthropic/Claude, Google/Gemini,
  and Ollama, embedded as defaults, validated against local fixtures, compiled
  into runtime security-event rules, and proven through MITM/session DB plus
  the `/security/{id}/latest` ledger surface.
- Auto-detected AI provider credentials/configs create or update endpoint
  settings/index metadata through the broker/security path; they do not copy
  tool config into a second provider-settings blob.
- Private VirusTotal-style file scanning as an import litmus case. This sprint builds the mandatory file import rail, not the scanner plugin itself.
- PII scanning as an export litmus case. This sprint builds the mandatory byte/text export rail, not the scanner plugin itself.
- Security event emission and DB writer batching/flush behavior.
- Launch/startup critical path.

## Current T6 Proof

- Provider identity now lives in settings/profile data through
  `ProviderRuleProfile::endpoint_registry()` and `ModelEndpointRegistry`.
- Rust parser selection uses `ModelProtocol` as a typed wire-protocol adapter:
  `anthropic`, `openai`, `google`, and native `ollama`.
- `ProviderKind` remains as a compatibility alias for existing code only.
- A custom OpenAI-compatible endpoint with `protocol = "openai-compatible"`
  maps to `ModelProtocol::OpenAi` without a new provider enum variant.
- Model endpoint records now carry provider identity/display name, protocol,
  upstream URL, aliases, listen ports, credential setting slot, optional
  `credential:blake3:` ref, allowed remote targets, and tool-owned config
  files. MITM classifies model traffic from the live endpoint registry by
  host-plus-port, so local Ollama aliases such as `local.ollama:11434` are
  settings data instead of MITM hardcoding.
- Native Ollama has explicit request metadata parsing, non-streaming usage
  extraction, LLM path gating, and a no-op SSE parser so it does not borrow
  OpenAI stream semantics.
- `MergedPolicies` now carries `model_endpoints`, and `capsem-process` wires it
  into `MitmProxyConfig` at startup and updates it on config reload.
- MITM resolves request host to `ModelProtocol` through the live endpoint
  registry by host-plus-port, then passes that typed metadata to enforcement,
  broker substitution, hooks, and telemetry.
- HTTP request credential materialization is owned by
  `security_engine::materialize_http_request_for_upstream` after action rules
  mutate the `SecurityEvent`. The MITM broker-substitution wrapper is removed;
  production MITM code has no second substitute/materialize API.
- The SSE parser and Anthropic/OpenAI/Google interpreter hooks no longer infer
  providers from hardcoded domains. Burn tests prove `api.openai.com` alone is
  ignored without runtime provider metadata.
- Native Ollama is proven through the MITM plain-HTTP path. A request to
  `/api/chat` with `Host: 127.0.0.1:<port>` resolves through settings-owned
  endpoint data to `ModelProtocol::Ollama`, emits a `model_calls` row with
  request metadata and non-streaming usage counts, and feeds a matching
  `model.call` security-rule ledger row.
- File boundary roots are now first-class event identities:
  `FileAction::Imported` emits `file.import`, `FileAction::Exported` emits
  `file.export`, and ordinary file create/write/read/delete still emit
  `file.event` while exposing their specific CEL roots. New rule/ask ledger
  tables reject stale event types such as `model.request`, `dns.response`, and
  `file.ingress`.
- Service workspace upload now emits `LogFileBoundary Import` through the
  process-owned ledger before writing bytes, and failed import ledger writes
  fail closed without creating the target file. Service workspace download
  emits `LogFileBoundary Export` before returning bytes. Legacy `/write_file`
  emits import before sending bytes to the guest, and guest `ReadFile`
  responses are emitted as `file.export` by `capsem-process` before the read
  job resolves.
- `fs_monitor` is audit/reconciliation-only. It may log matching rules,
  including `action = "block"`, but those rows are `file.event` detections and
  never `file.import` / `file.export` boundary gates.
- Old frontend setup assumptions are burned from the settings surface:
  `needsSetup`, missing API-key setup warnings, password "required" badges, and
  the frontend `validateApiKey` helper are removed. `load_settings_response`
  exposes provider status and tool-owned config source indexes, and the AI
  settings page renders configured/discovered providers, brokered
  `credential:blake3:` refs, corp block state, and indexed tool config
  sources.
- Provider discovery records are first-class settings metadata under
  `[ai.<provider>.discovery]`. Credential brokerage writes the credential ref
  setting and provider discovery record atomically for built-in AI providers.
  Discovery records may carry only canonical runtime event types and
  `credential:blake3:` references; raw secrets and stale event-type names are
  rejected by the provider-profile contract. Discovery-only user records merge
  against built-in provider defaults, so they do not copy endpoint/rule config
  or create a second provider registry.
- Tool-owned config source indexes are first-class metadata under
  `[tool_config_sources.<id>]`. They record the tool id, guest path, typed
  format, optional `blake3:<hex>` observed hash/version, inferred `ai.<id>`
  endpoint ref, credential refs, and typed allowed overlays. The loader rejects
  rendered config content, raw credentials, malformed hashes, and malformed
  endpoint refs.
- Stored provider credential settings must be empty or `credential:blake3:`
  references. The loader and batch-update API reject raw API keys/tokens for
  AI and GitHub credential setting ids, so credential brokerage is the settings
  write path for secrets.
- Provider-owned rules compile directly to deterministic security-event rule
  ids such as `profiles.rules.ai_openai_http_api`. They carry `action`,
  optional `detection_level`, priority/corp-lock metadata, and plugin config for
  preprocess/postprocess actions. Provider rules do not generate old Policy V2
  callback rules.
- Provider discovery and user-authored provider allow rules cannot re-enable a
  corp-blocked provider. The merged rule set dedupes by deterministic provider
  rule id and applies corp after user/default rules, preserving the corp block,
  negative priority, detection level, and corp-lock metadata.
- Focused runtime proof passed:
  `provider_profile`, `merged_policies_carry_live_model_endpoint_registry`,
  `model_provider_routing_uses_live_endpoint_registry`, `sse_parser_hook`,
  `interpreter_hook`,
  `ollama_settings_endpoint_routes_and_emits_model_call_security_event`,
  `cargo test -p capsem-service security_latest_returns_full_session_db_rule_ledger_rows -- --nocapture`,
  `cargo test -p capsem-service logs_file -- --nocapture`,
  `cargo test -p capsem-service import_ledger_fails -- --nocapture`,
  `cargo test -p capsem-service write_file_logs_import_before_guest_write -- --nocapture`,
  `cargo test -p capsem-process read_file_content_emits_file_export_before_job_result -- --nocapture`,
  `cargo test -p capsem-core fs_monitor -- --nocapture`,
  `cargo test -p capsem-core provider_discovery -- --nocapture`,
  `cargo test -p capsem-core settings_file_parses_discovery_only_provider_record -- --nocapture`,
  `cargo test -p capsem-core tool_config_source_index -- --nocapture`,
  `cargo test -p capsem-core raw_provider_credentials -- --nocapture`,
  `cargo test -p capsem-core policy_config -- --nocapture`,
  `cargo test -p capsem-core provider_profile -- --nocapture`,
  `cargo test -p capsem-core model_provider_routing_uses_live_endpoint_registry -- --nocapture`,
  `cargo test -p capsem-core merged_policies_carry_live_model_endpoint_registry -- --nocapture`,
  `cargo test -p capsem-core materializer -- --nocapture`,
  `cargo test -p capsem-core policy_v2_builtin_broker_action_materializes_upstream_and_logs_reference_only -- --nocapture`,
  `cargo test -p capsem-core provider_discovery_and_user_allow_cannot_reenable_corp_blocked_provider -- --nocapture`,
  `cargo test -p capsem-core load_settings_response_exposes_provider_and_tool_config_status -- --nocapture`,
  `pnpm --dir frontend check`,
  `pnpm --dir frontend test`,
  `cargo test -p capsem-logger schema -- --nocapture`,
  `cargo test -p capsem-core security_engine -- --nocapture`,
  `cargo check -p capsem-process`, stale `detect_ai_provider` burn search, and
  `git diff --check`.

## Output Artifacts

- `capsem-debug-upstream` test binary/server with local endpoints.
- `capsem-bench mitm-local` benchmark mode.
- OTEL/tracing span map for MITM, security event emission, DB writer, and launch.
- Replacement tests for network diagnostics and MITM tests that currently depend on external services.
- Benchmark JSON under `benchmarks/mitm-local/` and launch/DB categories where appropriate.
- A final hotspot report with numbers and a yes/no recommendation for any follow-up optimization sprint.
- Hotspot report:
  `sprints/perf-observability-network-lab/hotspot-report.md`.

## Current Benchmark Proof

- DB writer pressure on this host, from
  `benchmarks/db-writer/data_1.0.1780763638_arm64.json`: 128-event bursts
  p50/p95/p99 1.5188/1.5538/1.5588 ms and 83.934K events/s mean;
  1024-event bursts 6.8931/7.0277/7.0382 ms and 148.063K events/s mean;
  4096-event bursts 27.0200/27.8743/28.0951 ms and 150.797K events/s mean.
- Local MITM network matrix is captured through the gated VM benchmark. The
  gate remains opt-in with `CAPSEM_RUN_MITM_LOCAL_BENCH=1` so normal tests do
  not boot a VM or depend on a routable local debug-upstream URL.
- VM/MITM local matrix archived at
  `benchmarks/mitm-local/data_1.0.1780763638_arm64.json` with 10 requests,
  concurrency 1:
  `tiny_http` p50/p95/p99 1.5/3.3/4.3 ms; `http_1mb` 14.6/15.9/16.1 ms;
  `gzip_1mb` 34.7/37.7/37.9 ms; `sse_model` 1.4/2.6/2.7 ms;
  `denied_target` 1.3/2.0/2.2 ms; `credential_response` 1.2/2.1/2.1 ms;
  `websocket_echo` 0.2/0.2/0.2 ms over 10 frames; `websocket_close`
  1.7/1.7/1.7 ms over one frame.
- Session DB proof is now part of the gated benchmark: expected HTTP and
  WebSocket paths, WebSocket status `101`, all allowed decisions, and no raw
  `capsem_test_` synthetic secret marker in audited text columns.
- Lifecycle benchmark archived at
  `benchmarks/lifecycle/data_1.0.1780763638.json`: total lifecycle
  p50/p95/p99 1057.0/1065.1/1065.8 ms; provision 973.2/982.1/982.9 ms;
  exec-ready 11.6/11.6/11.6 ms; exec 11.3/11.4/11.4 ms; delete
  60.0/61.0/61.1 ms.

## Release Hold

Active until:

- Debug OTEL cannot leak raw credential material.
- Old provider setup/onboarding UI and raw provider API-key forms are gone; the
  settings view renders endpoint/provider records, broker refs, and discovered
  tool config sources.
- Restore/snapshot paths remain classified as lifecycle state movement unless
  future user import/export behavior is added.

Cleared release-hold item:

- Local lab starts and stops deterministically in tests and as a binary smoke.
  It currently supports local plain HTTP plus WebSockets; HTTPS remains outside
  the local upstream until a production-safe deterministic trust path exists.
- Default network tests no longer depend on public HTTP/DNS/provider probes
  unless the explicit public-smoke flag is set.
- WebSocket upgrades are locally proven through MITM with a `101` tunnel and
  an allowed session-DB event.
- VM/MITM local benchmark JSON is archived and tied to session DB event proof.
- DB write pressure has numbers and no event-loss path in the logger-owned
  writer benchmark.
- Launch timing has lifecycle numbers and an archived JSON artifact.
- The old MITM hardcoded provider classification path is removed; runtime
  provider classification is endpoint-registry based and covered by burn tests.
- Direct service upload/download and legacy write/read file boundary paths are
  gated through the process-owned `file.import` / `file.export` security-event
  rail and covered by focused burn tests.
- `fs_monitor` audit/reconciliation rows are proven to stay on `file.event`;
  they are not treated as import/export enforcement proof.
- Provider/broker settings UI is wired to the backend settings contract and is
  covered by focused backend and frontend gates.
- Endpoint routing and HTTP credential materialization are proven to flow
  through the endpoint registry and security engine, not a MITM helper rewrite.

## Swarm Process

- Control board: `swarm.md`.
- Completed finding doc: `swarm-findings/agent-vault.md`.
- Completed evidence: local clone review against upstream revision
  `234dbf0d27d4749b35690c91713fd2789c810cd7`; Firecrawl
  (`019e99a0-30ac-7212-a4d3-81d1b362c11d`) was repeatedly polled but not
  relied on.
- Accepted T6 patterns: explicit broker substitution surfaces,
  injected-auth-wins/header-strip invariants, no-raw-secret tests across all
  sinks, credential-missing diagnostics, scoped runtime identity fields, and
  actionable denial metadata.
- Rejected ports: a second service-rule matcher, separate request-log sink,
  proposal engine as authority, or settings/session DB secret storage.

## Litmus Test

The litmus test for this sprint:

1. Start a fresh VM.
2. Start the local debug upstream.
3. Run local HTTP, gzip, SSE/model-like, WebSocket, credential-broker, and denial cases.
4. Query spans/metrics and `session.db`.
5. Produce a single table:

DB columns use the archived logger-owned writer benchmark above. They prove the
single SQLite writer can absorb these event rates, while live per-request
debug-metric export remains a future local OTEL/debug endpoint task. Do not
route these counters through `/status`.

| Case | p50 | p95 | p99 | DB enqueue | DB batch/write | Security events emitted | Raw secret leaked? |
| --- | ---: | ---: | ---: | --- | --- | ---: | --- |
| tiny HTTP | 1.5 ms | 3.3 ms | 4.3 ms | metric wired | 128-event burst p50/p95/p99 1.5188/1.5538/1.5588 ms | 10 `net_events` | no |
| 1 MB HTTP | 14.6 ms | 15.9 ms | 16.1 ms | metric wired | 128-event burst p50/p95/p99 1.5188/1.5538/1.5588 ms | 10 `net_events` | no |
| gzip 1 MB | 34.7 ms | 37.7 ms | 37.9 ms | metric wired | 128-event burst p50/p95/p99 1.5188/1.5538/1.5588 ms | 10 `net_events` | no |
| SSE model stream | 1.4 ms | 2.6 ms | 2.7 ms | metric wired | 128-event burst p50/p95/p99 1.5188/1.5538/1.5588 ms | 10 `net_events` | no |
| WebSocket echo | 0.2 ms | 0.2 ms | 0.2 ms | metric wired | 128-event burst p50/p95/p99 1.5188/1.5538/1.5588 ms | 1 `net_event` | no |
| denied request | 1.3 ms | 2.0 ms | 2.2 ms | metric wired | 128-event burst p50/p95/p99 1.5188/1.5538/1.5588 ms | 10 `net_events` | no |
| credential capture | 1.2 ms | 2.1 ms | 2.1 ms | metric wired | 128-event burst p50/p95/p99 1.5188/1.5538/1.5588 ms | 10 `net_events` | no |
| file import gate | TBD | TBD | TBD | TBD | TBD | TBD | no |
| file export gate | TBD | TBD | TBD | TBD | TBD | TBD | no |

If that table cannot be produced, we are not ready to optimize.
