# Sprint: perf-observability-network-lab

## Tasks

- [x] T0.1 -- Create sprint control docs.
- [x] T0.2 -- Freeze span naming and allowed labels.
- [x] T0.3 -- Inventory current network tests and mark each as local-lab replacement, explicit external smoke, or obsolete.
- [x] T0.4 -- Create real provider-rule draft TOML for OpenAI/Codex, Anthropic/Claude, Google/Gemini, and Ollama.
- [x] T0.5 -- Validate provider-rule draft against code fixtures, local events, the security latest endpoint, and `session.db` expectations.
- [x] T0.5a -- Parse and compile provider-rule draft with focused Rust tests.
- [x] T0.5b -- Exercise provider-rule conditions through the shared CEL condition evaluator helper.
- [x] T0.5d -- Prove every provider-rule draft condition with positive and negative security-event fixtures.
- [x] T0.5c -- Validate compiled provider rules against runtime security latest endpoint and `session.db` events.
- [x] T0.5e -- Fix full-package test isolation around credential broker HOME/CAPSEM env state.
- [x] T0.5f -- Embed provider-owned defaults for OpenAI/Codex, Anthropic/Claude, Google/Gemini, and Ollama.
- [x] T0.5g -- Compile provider-owned defaults plus user/corp overlays into runtime Policy V2 HTTP/DNS/model callbacks and settings response.
- [x] T0.5h -- Capture Infisical `agent-vault` review findings into `swarm-findings/agent-vault.md`.
- [x] T1.1 -- Build local debug upstream with HTTP deterministic endpoints.
- [x] T1.2 -- Add gzip, slow chunk, SSE/model-like, and credential response endpoints.
- [x] T1.3 -- Add WebSocket echo/ping/close endpoints.
- [x] T1.4 -- Add lifecycle helper so tests can start/stop the debug upstream deterministically.
- [x] T2.1 -- Add debug-only OTEL/tracing config that cannot export upstream by default.
- [x] T2.2 -- Add MITM request spans around protocol, TLS, policy, actions, upstream, response policy, model policy, and chunk hooks.
- [x] T2.3 -- Add security-event emit span/metrics with canonical event type/family labels.
- [x] T2.4 -- Add DB writer enqueue, batch, batch-size, and shutdown-flush metrics.
- [x] T2.5 -- Add launch spans for service, gateway, process spawn, VM boot, vsock ready, and first network request.
- [x] T3.1 -- Add `capsem-bench mitm-local`.
- [x] T3.2 -- Add host-side local MITM benchmark artifact writer.
- [x] T3.3 -- Add DB writer pressure benchmark.
- [x] T3.4 -- Add launch timing benchmark or extend existing lifecycle benchmark with span-backed breakdowns.
- [x] T4.1 -- Replace public HTTP/proxy throughput diagnostics with local lab.
- [x] T4.2 -- Replace public default `capsem-bench http`/`throughput` targets with local-lab-backed targets where run under Capsem.
- [x] T4.3 -- Keep public-network checks only as explicit smoke tests.
- [x] T4.4 -- Add WebSocket E2E tests through Capsem network path.
- [x] T5.1 -- Run local benchmark matrix and archive JSON.
- [x] T5.2 -- Query `session.db` to prove expected security events and no raw secret leakage.
- [x] T5.3 -- Fill litmus table in `MASTER.md`.
- [x] T5.4 -- Write hotspot report and optimization recommendation.
- [x] T6.1 -- Replace closed provider-enum assumptions in the sprint target with settings-defined endpoint/provider identity.
- [x] T6.2 -- Define `ModelProtocolAdapter` registry boundary for OpenAI/Anthropic/Google/Ollama wire formats; dynamic plugins are deferred until after 1.3.
- [x] T6.3 -- Define settings schema for model endpoints: protocol, provider identity, aliases, listen ports, upstream URL, credential slot/ref, allowed remote target.
- [x] T6.4 -- Prove custom OpenAI-compatible endpoint requires settings only, not core provider edits.
- [x] T6.5 -- Prove Ollama endpoint routing uses settings plus protocol adapter and emits canonical `model.call`.
- [x] T6.6 -- Prove endpoint routing/materialization flows through security events/actions, not MITM helper rewrites.
- [x] T6.7 -- Burn the old closed provider system: remove direct hardcoded provider matching or make it private registry bootstrap only.
- [x] T6.8 -- Add burn guards so new providers cannot require central enum/path edits.
- [x] T6.9 -- Define mandatory `file.import.*` and `file.export.*` security-event contract for VM/workspace boundary bytes.
- [x] T6.10 -- Inventory and gate service upload, API/CLI write_file, and file download/export paths; restore/snapshot remains non-user-byte movement unless later import/export behavior is added.
- [x] T6.11 -- Prove `fs_monitor` is audit/reconciliation only and not treated as enforcement proof.
- [x] T6.12 -- Add burn guards/tests so direct boundary byte writes fail without security-event gate coverage.
- [x] T6.13 -- Burn old provider setup UI assumptions: settings UI displays configured/detected providers and broker status, not install/onboarding provider forms.
- [x] T6.14 -- Define provider discovery settings patches from credential/OAuth/tool-config observations.
- [x] T6.15 -- Define tool-owned config source index records: tool id, guest path, format, observed hash/version, inferred endpoint ref, credential refs, and allowed Capsem overlays.
- [x] T6.16 -- Add burn guards so config materialization cannot store raw provider credentials or rendered config content in settings, and cannot bypass security-event/file materialization.
- [x] T6.17 -- Define provider-owned rules such as `[ai.openai.rules.detect]`, `[ai.openai.rules.replace]`, and `[ai.openai.rules.block]`, compiled into deterministic CEL/Sigma engine rules with priorities and corp-lock metadata.
- [x] T6.18 -- Prove corp-disabled providers cannot be auto-added as usable endpoints and cannot be re-enabled by user discovery/settings.
- [x] Final Gate -- `cargo fmt --check`.
- [x] Final Gate -- focused Rust tests for local lab and spans.
- [x] Final Gate -- focused Python/VM tests for local network replacement.
- [x] Final Gate -- benchmark JSON archived.

## Notes

- Discovery: Existing MITM path already records `mitm.tls_handshake_ms`, `mitm.upstream_dial_ms`, and per-hook `mitm.hook_duration_ms`, but the local deterministic server and end-to-end span correlation are missing.
- Discovery: Current HTTP benchmark artifacts mix Capsem overhead with public network variability.
- Decision: Debug OTEL is local/debug-only. No upstream exporter is enabled by default.
- Decision: Optimization is deferred until T5 numbers exist.
- Decision: Model provider/endpoints belong in settings as data. Rust owns protocol parser adapters/registries and generic routing/materialization machinery.
- Decision: Custom OpenAI-compatible endpoints must use the existing OpenAI protocol adapter with a settings-defined endpoint/provider identity.
- Decision: No long-lived old provider path. The registry replaces the wheel; it does not sit beside it.
- Completed: MITM model provider classification now snapshots the live
  `ModelEndpointRegistry` from merged settings. `MitmProxyConfig` carries the
  registry beside Policy V2, and `capsem-process` reload updates it with the
  rest of the live policy state.
- Completed: The SSE parser and provider interpreter hooks no longer infer AI
  providers from domain names. They trust only `ConnMeta.ai_provider`, which is
  seeded by MITM from the endpoint registry.
- Completed: Native Ollama is now proven through the production MITM plain-HTTP
  path: `Host: 127.0.0.1:<port>` resolves through the live endpoint registry
  to `ModelProtocol::Ollama`, `/api/chat` dispatches upstream, the native
  request/usage parser fills `model_calls`, and a matching rule records a
  canonical `model.call` ledger row.
- Verification: focused provider/runtime burn gates passed:
  `provider_profile`, `merged_policies_carry_live_model_endpoint_registry`,
  `model_provider_routing_uses_live_endpoint_registry`, `sse_parser_hook`, and
  `interpreter_hook`.
- Verification: `cargo test -p capsem-core ollama_settings_endpoint_routes_and_emits_model_call_security_event -- --nocapture`
  passed, including `model_calls.event_id` and `security_rule_events.event_type
  = "model.call"` assertions.
- Completed: Model endpoint schema now carries provider id/name, protocol,
  upstream URL, aliases, listen ports, credential setting slot, optional
  `credential:blake3:` ref, allowed remote targets, and tool-owned config
  files. MITM provider classification uses the endpoint registry by
  host-plus-port for request traffic. Default Ollama now includes
  `local.ollama`/localhost aliases on port `11434`; custom
  OpenAI-compatible endpoints can define their own aliases and ports without
  Rust provider enum growth.
- Verification: endpoint schema and target-routing gates passed:
  `cargo test -p capsem-core provider_profile -- --nocapture`,
  `cargo test -p capsem-core model_provider_routing_uses_live_endpoint_registry -- --nocapture`,
  `cargo test -p capsem-core merged_policies_carry_live_model_endpoint_registry -- --nocapture`,
  `cargo test -p capsem-core ollama_settings_endpoint_routes_and_emits_model_call_security_event -- --nocapture`,
  `cargo test -p capsem-core load_settings_response_exposes_provider_and_tool_config_status -- --nocapture`,
  `pnpm --dir frontend check`, and `pnpm --dir frontend test`. One parallel
  Rust attempt hit macOS `run_signed.sh` codesign contention; the same test
  passed when rerun alone.
- Completed: Endpoint routing/materialization is on one rail. MITM provider
  routing uses the live endpoint registry by host-plus-port; request
  credential materialization goes through
  `security_engine::materialize_http_request_for_upstream` after action rules
  mutate the `SecurityEvent`. The old MITM broker-substitution wrapper was
  removed so production code has no second substitute/materialize API.
- Verification: T6.6 proof passed:
  `cargo test -p capsem-core materializer -- --nocapture`,
  `cargo test -p capsem-core policy_v2_builtin_broker_action_materializes_upstream_and_logs_reference_only -- --nocapture`,
  and burn search
  `rg -n "substitute_brokered_upstream_credentials\\(|materialize_http_request_for_upstream\\(" crates/capsem-core/src crates/capsem-process/src crates/capsem-service/src -g '*.rs'`.
  The search shows production broker substitution only in
  `credential_broker`/`security_engine`, with MITM production calling only the
  security-engine materializer; direct broker calls remain test-only.
- Completed: `file.import` and `file.export` are now closed
  `RuntimeSecurityEventType` variants. `FileAction::Imported` and
  `FileAction::Exported` emit those canonical event types into security-rule
  ledger rows; ordinary create/write/read/delete continue to emit
  `file.event` with their specific CEL roots.
- Completed: New `security_rule_events` and `security_ask_events` tables now
  reject unknown/stale event types such as `dns.response`, `model.request`, and
  `file.ingress`.
- Verification: `cargo test -p capsem-logger schema -- --nocapture` passed with
  strict event-type CHECK coverage for rule and ask ledgers.
- Verification: `cargo test -p capsem-core security_engine -- --nocapture`
  passed with file import/export event-type mapping, CEL roots, and ledger
  assertions.
- Verification: `cargo check -p capsem-process` passed after threading the
  model endpoint registry through startup and live reload.
- Verification: burn search found no remaining `detect_ai_provider` helper in
  MITM/process provider-routing code, and `git diff --check` passed.
- Completed: Service workspace upload now emits `LogFileBoundary Import`
  through the process-owned ledger before writing bytes; failed import ledger
  writes fail closed and leave no workspace file behind. Service download emits
  `LogFileBoundary Export` before returning bytes. Legacy `/write_file` emits
  import before sending `WriteFile` to the guest, and guest `ReadFile`
  responses are emitted as `file.export` by `capsem-process` before resolving
  the read job.
- Verification: focused file-boundary burn gates passed:
  `cargo test -p capsem-service logs_file -- --nocapture`,
  `cargo test -p capsem-service import_ledger_fails -- --nocapture`,
  `cargo test -p capsem-service write_file_logs_import_before_guest_write -- --nocapture`,
  and
  `cargo test -p capsem-process read_file_content_emits_file_export_before_job_result -- --nocapture`.
- Completed: `fs_monitor` remains an audit/reconciliation producer. Matching
  rules, including `action = "block"`, are recorded in `security_rule_events`
  as `file.event`; they do not become `file.import` / `file.export` boundary
  gates and do not materialize or veto user byte movement.
- Verification: `cargo test -p capsem-core fs_monitor -- --nocapture` passed
  with 18 focused tests, including the audit-only block-rule burn guard.
- Completed: Frontend setup/onboarding assumptions are burned from the settings
  surface. `SettingsModel.needsSetup`, store-level `needsSetup`, missing API-key
  setup warnings, password "required" badges, and the frontend
  `validateApiKey` helper are removed.
- Completed: `load_settings_response` now exposes provider status and
  tool-owned config source indexes directly from the settings contract. The AI
  settings page renders configured/discovered providers, brokered
  `credential:blake3:` refs, corp block state, and indexed tool config
  sources instead of setup/onboarding provider forms.
- Verification: `cargo test -p capsem-core load_settings_response_exposes_provider_and_tool_config_status -- --nocapture`
  passed. `pnpm --dir frontend check` and `pnpm --dir frontend test` passed
  with 0 errors, 0 warnings, 0 hints, and 361 unit tests.
- Completed: Provider discovery is now a typed settings patch. Credential
  brokerage writes the broker ref setting and `[ai.<provider>.discovery]`
  atomically for built-in AI providers; discovery records carry source,
  canonical runtime event type when available, confidence, trace id, observed
  timestamp, and a `credential:blake3:` ref. Raw secrets and stale event-type
  names are rejected at the provider-profile contract.
- Verification: provider discovery focused gates passed:
  `cargo test -p capsem-core provider_discovery -- --nocapture`,
  `cargo test -p capsem-core settings_file_parses_discovery_only_provider_record -- --nocapture`,
  `cargo fmt -p capsem-core --check`, and `git diff --check`.
- Completed: Tool-owned config source metadata is now typed under
  `[tool_config_sources.<id>]`. Records carry `tool_id`, `guest_path`,
  `format`, optional `observed_hash = "blake3:<hex>"`, optional
  `observed_version`, optional `inferred_endpoint_ref = "ai.<provider>"`,
  broker credential refs, and typed allowed overlays. `load_settings_file`
  validates the metadata contract.
- Verification: `cargo test -p capsem-core tool_config_source_index -- --nocapture`
  passed, proving valid source indexes load/roundtrip and invalid raw
  credentials, rendered `content`, malformed hashes, and malformed endpoint
  refs are rejected.
- Completed: Raw provider credentials are no longer valid stored settings for
  AI/GitHub credential setting ids. Stored values must be empty or
  `credential:blake3:` references, and both `load_settings_file` and
  `batch_update_settings*` enforce that contract. Old raw-key test fixtures
  were burned to broker refs.
- Fixed: policy_config env-mutating tests now share the credential-broker test
  env lock, removing a parallel `CAPSEM_*_CONFIG` race exposed by the burn
  pass.
- Verification: `cargo test -p capsem-core raw_provider_credentials -- --nocapture`
  and `cargo test -p capsem-core policy_config -- --nocapture` passed
  (`464` policy_config tests).
- Completed: Provider-owned rules compile to deterministic security-event rule
  ids (`profiles.rules.ai_<provider>_<rule>`), not generated Policy V2
  callbacks. The contract covers allow/block plus preprocess/postprocess plugin
  actions, `detection_level`, user/corp priority defaults, and corp-lock
  metadata.
- Verification: `cargo test -p capsem-core provider_profile -- --nocapture`
  passed with the provider-owned rule contract and old Policy V2 callback burn
  guard.
- Completed: User provider discovery and user allow rules cannot re-enable a
  corp-blocked provider. Merged security rules dedupe by deterministic provider
  rule id, then corp rules replace user/default rules and retain block action,
  negative priority, and corp-lock metadata.
- Verification: `cargo test -p capsem-core provider_discovery_and_user_allow_cannot_reenable_corp_blocked_provider -- --nocapture`
  passed, and the full `cargo test -p capsem-core policy_config -- --nocapture`
  gate passed with `466` tests.
- Decision: Scanner plugins are deferred until after 1.3, but their foundation is mandatory now: all VM/workspace byte import and export must pass through first-party security events before boundary materialization.
- Decision: The old AI setup/onboarding UI is not a provider authority. Provider settings come from defaults, explicit settings edits, or security-path discovery. The UI renders those records and broker status.
- Decision: Provider/tool config files remain tool-owned source-of-truth files. Capsem settings store endpoint records, broker refs, discovery/index metadata, and narrow overlays only; no second full config copy.
- Decision: Provider and credential rules are authored under provider/profile objects, not as user-facing top-level `policy.*` stanzas. The engine may compile them to internal deterministic CEL/Sigma rules. No separate provider matcher schema.
- Decision: Provider detect and corp block share named rules and conditions;
  corp disables by flipping `decision` to `block`, using negative priority, and
  locking the rule. No separate `enabled` gate is required for provider access.
- Decision: Provider profile priority invariant is now executable: non-corp
  rules cannot use negative priority; corp-locked rules may only use negative
  or zero priority.
- Decision: Debug spans are a stable contract. Span names and labels are
  frozen in `T0-contract.md`; labels must be low-cardinality enum/bucket values
  and must never include raw hostnames, URLs, paths, bodies, cookies, OAuth
  tokens, API keys, or raw credentials.
- Decision: Default network correctness/performance gates must move to the
  deterministic local lab. Public-network checks may remain only as explicit
  smoke tests.
- Decision: VM/workspace boundary roots are `file.import` and `file.export`.
  Do not resurrect `file.ingress` / `file.egress` as user-facing CEL roots.
- Completed: T0 provider-rule draft parses as TOML and covers OpenAI/Codex,
  Anthropic/Claude, Google/Gemini, and Ollama with 30 small provider-owned
  rules. Runtime provider-rule validation now installs the real compiled
  provider defaults in the MITM path and proves Ollama HTTP/model rule ledger
  rows in `session.db`.
- Verification: T0.5c runtime/provider-rule proof passed:
  `cargo test -p capsem-core ollama_settings_endpoint_routes_and_emits_model_call_security_event -- --nocapture`
  and
  `cargo test -p capsem-service security_latest_returns_full_session_db_rule_ledger_rows -- --nocapture`.
- Completed: `ProviderRuleProfile` parser/compiler added under
  `capsem_core::net::policy_config`. It validates no disjunctive mega-rules,
  enforces negative priority as corp-only, compiles deterministic rule ids, and
  exposes `evaluate_provider_rule_condition` so tests use the shared CEL
  condition evaluator.
- Completed: The provider-rule draft now has table-driven positive and negative
  security-event fixtures for all 30 rules, including HTTP, DNS, file import,
  credential observation/replacement, model request, and Ollama local endpoint
  shapes.
- Completed: Provider-owned defaults are embedded in
  `crates/capsem-core/src/net/policy_config/default_provider_rules.toml`.
  Empty user/corp settings now still produce provider Policy V2 HTTP, DNS, and
  model request rules, while user and corp TOML can override the same
  provider/rule records. Corp overrides win by replacing the same rule name,
  not by creating a competing second rule.
- Discovery: Provider-owned `credential.observed` and `file.import`
  rules are parsed and CEL-fixture tested, but they do not yet compile into
  Policy V2 runtime callbacks because those callback families do not exist
  there yet. HTTP/DNS/model provider rules are runtime-wired.
- Swarm: `Infisical/agent-vault` prior-art review launched as Erdos
  (`019e999f-7099-7fc2-824d-6595ee10373f`) and tracked in `swarm.md`.
- Completed: `Infisical/agent-vault` prior-art review captured in
  `sprints/perf-observability-network-lab/swarm-findings/agent-vault.md`
  against upstream revision `234dbf0d27d4749b35690c91713fd2789c810cd7`.
  Accepted conceptual ports for T6: explicit broker substitution surfaces,
  injected-auth-wins/header-strip invariants, no-raw-secret tests across all
  sinks, credential-missing diagnostics, scoped runtime identity fields, and
  actionable denial metadata. Explicitly rejected ports: a second service-rule
  matcher, separate request-log sink, proposal engine as authority, or
  settings/session DB secret storage.
- Fixed: `mcp::tests::tool_cache_missing_file_returns_empty` leaked `HOME` as
  `/nonexistent_test_dir_xyz`, which could race credential-broker/security-engine
  tests under full `capsem-core`. It now uses an env guard and the shared test
  env lock.
- Verification: `cargo test -p capsem-core provider_profile -- --nocapture`
  passed with 11 focused tests, including built-in defaults and runtime Policy
  V2 compilation.
- Verification: focused settings/runtime provider gates passed:
  `merged_policies_include_builtin_provider_rules_without_user_toml`,
  `load_settings_response_includes_builtin_provider_policy`, and
  `merged_policies_compile_provider_rules_and_corp_block_wins`.
- Verification: `cargo test -p capsem-core policy_config -- --nocapture`
  passed with 438 tests.
- Verification: `cargo test -p capsem-core policy_v2_ -- --nocapture` passed
  with 77 tests across HTTP, DNS, MCP, model request/response, tool-call
  enforcement, rewrites, and fail-closed behavior.
- Verification: `cargo test -p capsem-core security_engine -- --nocapture`
  passed with 13 tests; `cargo test -p capsem-core credential_broker -- --nocapture`
  passed with 7 tests after rerunning alone to avoid macOS codesign wrapper
  contention.
- Verification: `cargo test -p capsem-logger -- --nocapture` passed with 101
  unit tests and 126 roundtrip integration tests.
- Verification: `cargo test -p capsem-core -- --nocapture` passed: 1848 unit
  tests, 26 MITM integration tests, 2 platform-gating tests, 12 settings-spec
  tests, and 11 VM integration tests; only explicit ignored tests remained
  ignored.
- Verification: `cargo test -p capsem-service settings -- --nocapture` passed
  with 7 service settings tests.
- Completed: T0 span contract now names MITM/network, security-event/DB, and
  launch spans with required labels and allowed values in
  `sprints/perf-observability-network-lab/T0-contract.md`.
- Completed: T0 network inventory captured in
  `sprints/perf-observability-network-lab/T0-network-test-inventory.md`,
  covering guest diagnostics, capsem-bench defaults, integration scripts,
  session tests, MCP builtin HTTP tests, provider smokes, and local-only tests.
- Completed: `capsem-debug-upstream` was added as a workspace binary/library
  under `crates/capsem-debug-upstream`. It binds `127.0.0.1:0` by default,
  prints one ready JSON object with the bound `base_url`, and exposes
  `/tiny`, `/bytes/{size}`, `/gzip/{size}`, `/sse/model`, `/slow-chunks`,
  `/credential/response`, `/echo`, `/deny-target`, `/ws/echo`, `/ws/ping`,
  and `/ws/close`.
- Completed: `spawn_debug_upstream()` returns a test lifecycle handle with
  `addr()`, `base_url()`, and `shutdown()`, so tests and benchmarks can start
  and stop the same server deterministically.
- Verification: `cargo check -p capsem-debug-upstream` passed.
- Verification: `cargo test -p capsem-debug-upstream -- --nocapture` passed
  with HTTP bytes/gzip, SSE model-like stream, secret-safe echo metadata, and
  WebSocket echo/ping/close coverage.
- Verification: `cargo run -p capsem-debug-upstream -- --addr 127.0.0.1:0`
  printed ready JSON for an ephemeral localhost port and stopped cleanly on
  Ctrl-C.
- Completed: `capsem_core::telemetry` now has a debug telemetry policy:
  `CAPSEM_DEBUG_TELEMETRY=local` widens local debug span filters, while OTLP
  exporter env vars are classified as blocked unless
  `CAPSEM_ALLOW_UPSTREAM_OTEL=true` is explicitly present. No upstream exporter
  is created by this switch.
- Verification: `cargo test -p capsem-core telemetry -- --nocapture` passed
  with 33 focused telemetry-related tests, including local-only debug policy,
  upstream-OTEL blocking, MITM telemetry, DNS telemetry, and selected
  security-rule ledger tests.
- Verification: `cargo check -p capsem-core` passed after rustfmt.
- Completed: MITM debug spans now use the stable contract names in
  `capsem_core::net::mitm_proxy::spans`: request, vsock classify, guest TLS
  handshake, request policy, security actions, model request policy, upstream
  prepare, upstream send, response policy, model response policy, and body
  chunk hooks. The request span no longer records raw domain/path fields.
- Verification: `cargo test -p capsem-core --lib span_names_match_capsem_mitm_contract -- --nocapture`
  passed.
- Verification: `cargo test -p capsem-core telemetry -- --nocapture` passed
  after the MITM span wiring, including the HTTP rewrite, model tool-call
  block, Ollama telemetry, and header-hashing MITM checks selected by the
  filter.
- Note: a parallel `cargo test` invocation for the span-name filter hit the
  macOS codesign wrapper while trying to launch the integration test binary.
  The same span contract was rerun with `--lib` and passed; no code change was
  needed for that harness contention.
- Completed: The unified security-event handoff now emits
  `capsem.security_event.emit` spans plus `security_event.emit_total` and
  `security_event.emit_duration_ms` metrics from
  `emit_security_write`/`emit_security_write_blocking`. Labels come from
  `RuntimeSecurityEventType::as_str()` and
  `RuntimeSecurityEventFamily::as_str()`.
- Verification: `cargo test -p capsem-core emit_security_write -- --nocapture`
  passed, including the new canonical emit metric assertion and the async/sync
  DB handoff tests.
- Completed: The logger-owned `DbWriter` now emits `capsem.db.enqueue`,
  `capsem.db.write_batch`, and `capsem.db.shutdown_flush` spans plus
  `db.enqueue_wait_ms`, `db.write_batch_total`,
  `db.write_batch_duration_ms`, `db.write_batch_size`, and
  `db.shutdown_flush_ms` metrics. The writer thread remains the sole SQLite
  writer.
- Verification: `cargo test -p capsem-logger db_writer_records -- --nocapture`
  passed with focused enqueue, batch, batch-size, and shutdown-flush metric
  assertions.
- Verification: `cargo check -p capsem-logger -p capsem-core` passed.
- Completed: Launch spans are wired for `capsem.launch.service`,
  `capsem.launch.gateway`, `capsem.launch.process_spawn`,
  `capsem.launch.vm_boot`, `capsem.launch.vsock_ready`, and
  `capsem.launch.first_network_ready`. The first network marker fires once per
  process on the first request reaching the MITM path.
- Verification: `cargo test -p capsem-core --lib launch_span_names_match_contract -- --nocapture`
  passed.
- Verification: `cargo check -p capsem-core -p capsem-service` passed.
- Completed: `capsem-bench mitm-local` now runs deterministic local-lab
  scenarios for tiny HTTP, 1 MB HTTP, gzip 1 MB, SSE/model stream,
  deny-target, credential response, WebSocket echo, and WebSocket close.
  The mode requires an explicit debug-upstream base URL and is never included
  in `capsem-bench all`.
- Completed: `tests/capsem-serial/test_mitm_local_benchmark.py` is a gated
  host-side artifact writer. With `CAPSEM_RUN_MITM_LOCAL_BENCH=1`, it starts
  or consumes a debug-upstream URL, provisions a VM, runs
  `capsem-bench mitm-local`, pulls `/tmp/capsem-benchmark.json`, asserts no
  synthetic raw API key is stored in the result JSON, and archives under
  `benchmarks/mitm-local/`.
- Completed: `crates/capsem-logger/benches/db_writer_pressure.rs` benchmarks
  the real `DbWriter` and SQLite schema with 128/1024/4096 file-event bursts.
  Archived run on this machine: 128-event bursts p50/p95/p99
  1.5188/1.5538/1.5588 ms and 83.934K events/s mean; 1024-event bursts
  6.8931/7.0277/7.0382 ms and 148.063K events/s mean; 4096-event bursts
  27.0200/27.8743/28.0951 ms and 150.797K events/s mean.
- Completed: The existing serial lifecycle benchmark now emits min/mean/p50/
  p95/p99/max operation summaries and carries the launch span contract in its
  JSON artifact so T5 can line it up with `capsem.launch.*` spans.
- Verification: `uv run pytest tests/test_capsem_bench_mitm_local.py -q`
  passed with 8 focused tests, including a real local HTTP fixture for
  tiny/1MB/gzip/SSE/deny/credential scenarios and secret-safe result JSON.
- Verification: `uv run pytest tests/capsem-serial/test_mitm_local_benchmark.py -q`
  skipped as designed without `CAPSEM_RUN_MITM_LOCAL_BENCH=1`, proving the
  benchmark writer is not part of the default test gate.
- Verification: `cargo bench -p capsem-logger --bench db_writer_pressure --no-run`
  passed.
- Verification: `cargo bench -p capsem-logger --bench db_writer_pressure -- --quiet`
  passed and produced the DB writer pressure numbers above.
- Verification: `uv run pytest tests/capsem-serial/test_lifecycle_benchmark.py --collect-only -q`
  collected both lifecycle benchmark tests after the artifact-schema extension.
- Verification: `python3 -m compileall -q tests/capsem-serial/test_lifecycle_benchmark.py tests/capsem-serial/test_mitm_local_benchmark.py guest/artifacts/capsem_bench/mitm_local.py guest/artifacts/capsem_bench/__main__.py`
  passed.
- Verification: `cargo fmt -p capsem-logger --check` and `git diff --check`
  passed.
- Completed: `capsem-bench http` no longer silently defaults to Google. It
  uses `CAPSEM_BENCH_MITM_LOCAL_BASE_URL/tiny` when present, uses the old
  public target only when `CAPSEM_BENCH_ALLOW_PUBLIC_NETWORK=1`, and otherwise
  returns a structured skipped result.
- Completed: `capsem-bench throughput` no longer silently defaults to the
  public PDF/CDN. It uses `CAPSEM_BENCH_MITM_LOCAL_BASE_URL/bytes/10mb` when
  present, uses the old public target only when
  `CAPSEM_BENCH_ALLOW_PUBLIC_NETWORK=1`, and otherwise returns a structured
  skipped result.
- Completed: The guest network diagnostic HTTP-port and throughput checks now
  prefer the local debug-upstream URL and otherwise require
  `CAPSEM_RUN_PUBLIC_NETWORK_SMOKE=1` before using Google/CDN public-network
  probes.
- Completed: T4.3 partial smoke sweep gated public DNS/TLS/curl/provider
  diagnostics in `test_network.py`, public DNS/allowed-domain checks in
  `test_sandbox.py`, and the Google AI domain diagnostic in `test_ai_cli.py`
  behind `CAPSEM_RUN_PUBLIC_NETWORK_SMOKE=1`.
- Completed: T4.3 MCP sweep gated positive public `fetch_http`, `grep_http`,
  and `http_headers` content diagnostics in `test_mcp.py` behind
  `CAPSEM_RUN_PUBLIC_NETWORK_SMOKE=1`. Blocked-domain and malformed-url tests
  remain default because they do not depend on public reachability.
- Completed: T4.4 replaced the old MITM WebSocket reject path with an
  HTTP/1.1 upgrade tunnel. The focused test uses a local upstream on
  `127.0.0.1`, proves `101 Switching Protocols`, relays `capsem-ws-ping` /
  `capsem-ws-pong` bytes through the upgraded stream, and asserts the session
  DB records one allowed `/ws` event with status `101`.
- Verification: `uv run pytest tests/test_capsem_bench_mitm_local.py tests/capsem-serial/test_mitm_local_benchmark.py -q`
  passed with 12 focused tests and 1 intended skip. The new tests prove local
  target selection and skip-without-public behavior for HTTP and throughput.
- Verification: `python3 -m compileall -q guest/artifacts/capsem_bench guest/artifacts/diagnostics/test_network.py tests/test_capsem_bench_mitm_local.py tests/capsem-serial/test_mitm_local_benchmark.py`
  passed.
- Verification: `python3 -m compileall -q guest/artifacts/diagnostics/test_network.py guest/artifacts/diagnostics/test_sandbox.py guest/artifacts/diagnostics/test_ai_cli.py guest/artifacts/capsem_bench tests/test_capsem_bench_mitm_local.py`
  passed after the T4.3 partial smoke sweep.
- Verification: `python3 -m compileall -q guest/artifacts/diagnostics/test_network.py guest/artifacts/diagnostics/test_sandbox.py guest/artifacts/diagnostics/test_ai_cli.py guest/artifacts/diagnostics/test_mcp.py guest/artifacts/capsem_bench tests/test_capsem_bench_mitm_local.py`
  passed after the MCP public-smoke gating.
- Verification: `cargo test -p capsem-core --lib websocket_upgrade_tunnels_through_local_upstream -- --nocapture`
  passed.
- Verification: `cargo check -p capsem-core` and
  `cargo fmt -p capsem-core --check` passed after the WebSocket tunnel work.
- Verification: `git diff --check` passed after the T4.1/T4.2 edits.
- Fixed: The gated VM `mitm-local` benchmark was initially a false positive:
  the guest had a stale initrd without the new mode, then arbitrary localhost
  debug-upstream ports bypassed transparent iptables, then WebSocket proxying
  attempted HTTP-proxy semantics. The harness now repacks current guest
  artifacts, writes an isolated `user.toml` allowing `127.0.0.1` plus the
  dynamic debug-upstream port through `security.web.http_upstream_ports`, uses
  explicit `127.0.0.1:10080` net-proxy env for HTTP, and gives WebSockets a
  pre-connected socket to the same net-proxy with `proxy=None`.
- Completed: T5.1 archived the real VM/MITM local benchmark at
  `benchmarks/mitm-local/data_1.0.1780763638_arm64.json`.
- Benchmark: T5.1 VM/MITM local matrix, 10 requests, concurrency 1:
  `tiny_http` p50 1.5 ms / p95 3.3 ms / p99 4.3 ms / 541.3 rps;
  `http_1mb` p50 14.6 ms / p95 15.9 ms / p99 16.1 ms / 68.5 rps /
  71.8 MB/s; `gzip_1mb` p50 34.7 ms / p95 37.7 ms / p99 37.9 ms /
  28.6 rps / 29.9 MB/s; `sse_model` p50 1.4 ms / p95 2.6 ms /
  p99 2.7 ms / 576.8 rps; `denied_target` p50 1.3 ms / p95 2.0 ms /
  p99 2.2 ms / 677.0 rps; `credential_response` p50 1.2 ms /
  p95 2.1 ms / p99 2.1 ms / 699.5 rps; `websocket_echo` 10 frames,
  p50/p95/p99 0.2 ms / 2456.0 fps; `websocket_close` 1 frame,
  p50/p95/p99 1.7 ms / 528.5 fps.
- Completed: T5.2 now queries the live session DB before teardown. It asserts
  at least 62 local MITM `net_events`, all expected HTTP/WebSocket paths,
  WebSocket `101` upgrade status, all `allowed` decisions, and no
  `capsem_test_` raw synthetic secret marker in request/response headers or
  body preview columns.
- Verification: `cargo test -p capsem-core http_upstream_ports -- --nocapture`
  passed with default, user override, and corp override coverage.
- Verification: `uv run pytest tests/test_capsem_bench_mitm_local.py -q`
  passed with 13 tests, including explicit WebSocket net-proxy socket coverage.
- Verification: `CAPSEM_RUN_MITM_LOCAL_BENCH=1 CAPSEM_BENCH_MITM_LOCAL_BASE_URL=http://127.0.0.1:50233 CAPSEM_BENCH_MITM_LOCAL_N=10 CAPSEM_BENCH_MITM_LOCAL_CONCURRENCY=1 uv run pytest tests/capsem-serial/test_mitm_local_benchmark.py -xvs`
  passed and archived the VM/MITM JSON.
- Completed: Launch lifecycle benchmark archived at
  `benchmarks/lifecycle/data_1.0.1780763638.json`.
- Benchmark: Lifecycle, 3 runs: `provision_ms` p50 973.2 / p95 982.1 /
  p99 982.9; `exec_ready_ms` p50/p95/p99 11.6; `exec_ms` p50 11.3 /
  p95 11.4 / p99 11.4; `delete_ms` p50 60.0 / p95 61.0 / p99 61.1;
  `total_ms` p50 1057.0 / p95 1065.1 / p99 1065.8.
- Verification: `uv run pytest tests/capsem-serial/test_lifecycle_benchmark.py::test_lifecycle_benchmark -xvs`
  passed.
- Completed: T5.4 hotspot report written at
  `sprints/perf-observability-network-lab/hotspot-report.md`. Recommendation:
  no broad 1.3 speed sprint; keep follow-up narrow to gzip-path profiling only
  if gzip-heavy workloads matter, and keep live debug metric export in the
  local OTEL/debug endpoint sprint rather than `/status`.
- Completed: T5.3 litmus table is filled in `MASTER.md`. DB columns reference
  the archived logger-owned writer benchmark at
  `benchmarks/db-writer/data_1.0.1780763638_arm64.json`; they do not pretend
  to be per-network-case runtime metric slices.
- Verification: `uv run pytest tests/test_archive_db_writer_benchmark.py -q`
  passed with parser and missing-Criterion-output coverage.
- Verification: `python3 scripts/archive_db_writer_benchmark.py` archived the
  DB writer Criterion results under `benchmarks/db-writer/`.
- Completed: T6.1/T6.2 split provider identity from parser protocol. New code
  should use `ModelProtocol` as the typed Rust wire adapter and keep provider
  identity in settings/profile data. The old `ProviderKind` name remains only
  as a compatibility alias for existing call sites.
- Completed: `ProviderRuleProfile::endpoint_registry()` builds a
  `ModelEndpointRegistry` from `[ai.<provider>]` data, including provider id,
  display name, typed protocol, upstream URL, and files.
- Completed: Native Ollama is an explicit `ModelProtocol::Ollama` slot with
  basic request metadata, non-streaming usage extraction, LLM path gating, and
  a no-op SSE parser so it does not borrow OpenAI stream semantics.
- Completed: T6.4 has a focused custom OpenAI-compatible endpoint test proving
  `protocol = "openai-compatible"` maps to the OpenAI adapter without adding a
  new Rust provider variant.
- Open: Runtime MITM provider detection still has direct domain matching in
  `mitm_proxy::detect_ai_provider` and the chunk hooks still carry
  `ProviderKind` through existing call sites. T6.6/T6.7/T6.8 must wire the
  endpoint registry into runtime routing and add burn guards before this lane
  is architecturally complete.
- Verification: `cargo test -p capsem-core provider_profile -- --nocapture`,
  `cargo test -p capsem-core model_protocol -- --nocapture`, and
  `cargo test -p capsem-core ollama -- --nocapture` passed.

## Coverage Ledger

- Unit/contract: Span-label and redaction contract is frozen in
  `T0-contract.md`; T2.1 local-only debug telemetry policy is covered by Rust
  tests. Span instrumentation starts in T2.2.
- Functional: Local HTTP/gzip/SSE/WebSocket endpoint tests now pass in
  `capsem-debug-upstream`; through-Capsem replacement tests start in T3/T4.
- Adversarial: Planned deny, malformed gzip, slow chunks, disconnect, WebSocket close, and credential leak tests.
- E2E/VM or integration: Gated host-side artifact writer for in-VM
  `capsem-bench mitm-local` now runs, asserts every scenario succeeds, checks
  session DB coverage, and archives JSON under `benchmarks/mitm-local/`.
- Telemetry/observability: T2 local-only debug policy, MITM request-path spans,
  security-event emit spans/metrics, DB writer metrics, and launch spans are
  wired. Through-Capsem benchmark capture remains T3/T5.
- Performance: DB writer pressure, VM/MITM network matrix, and lifecycle
  artifacts are archived. DB writer burst p50/p95/p99 are captured in
  `benchmarks/db-writer/data_1.0.1780763638_arm64.json`.
- Boundary architecture: Planned Ollama + custom OpenAI-compatible endpoint litmus, provider discovery/config-source DRY burn guards, plus file import/export rail for future private VT and PII scanner actions.
- Provider profile: Built-in defaults added for OpenAI/Codex,
  Anthropic/Claude, Google/Gemini, and Ollama; parser/compiler/CEL fixture
  validation passes; default/user/corp provider rules compile into runtime
  security-event rules and the settings response. Runtime MITM proof now shows
  provider-owned Ollama HTTP/model rule rows in `session.db`, and the service
  `/security/{id}/latest` endpoint returns the full DB-backed ledger shape.
- Final verification: `cargo fmt --check`; focused Rust gates
  `cargo test -p capsem-debug-upstream -- --nocapture`,
  `cargo test -p capsem-core websocket_upgrade_tunnels_through_local_upstream -- --nocapture`,
  `cargo test -p capsem-core --lib telemetry -- --nocapture`, and
  `cargo test -p capsem-logger db_writer_records_enqueue_batch_and_shutdown_metrics -- --nocapture`;
  focused Python gates
  `uv run python -m pytest tests/test_capsem_bench_mitm_local.py -v --tb=short`
  and
  `uv run python -m pytest tests/capsem-serial/test_mitm_local_benchmark.py -v --tb=short`
  (designed skip without `CAPSEM_RUN_MITM_LOCAL_BENCH=1`); benchmark JSON
  artifacts parsed successfully under `benchmarks/mitm-local/`,
  `benchmarks/db-writer/`, and `benchmarks/lifecycle/`.
- Test isolation: Full-package `capsem-core` now passes after fixing one
  leaking `HOME` test in MCP cache coverage.
- Missing/deferred: Actual optimization patches are deferred until a future
  optimization sprint. Live per-request metric export should be done through
  the planned local OTEL/debug endpoint, not `/status`.
