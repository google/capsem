# Sprint: security-event-rule-spine

## Tasks

- [x] T0.0 -- Create clean sprint docs for unified SecurityEvent rule spine.
- [x] T0.1 -- Define final TOML schema: first-principle storage in `corp.rules` and `profiles.rules`, with provider convenience normalized into profile rules.
- [x] T0.2 -- Add rule name validation: lowercase, max 64, `a-z0-9_-`, no spaces.
- [x] T0.3 -- Add action enum: `allow`, `ask`, `block`, `preprocess`, `postprocess`.
- [x] T0.4 -- Add detection level enum as optional metadata; accept `info` as sugar for `informational`, reject `level` and `action = "detect"`.
- [x] T0.5 -- Add priority source defaults: corp `-10`, built-in `0`, user/plugin `10`.
- [x] T0.6 -- Reject invalid explicit priorities for each source class; allow corp `<= -10`, built-in `0`, user/plugin `>= 10`.
- [x] T0.7 -- Require plugin for plugin-owned postprocess/preprocess rules.
- [x] T0.8 -- Add fixture TOML for detection metadata, broker, PII, VT, allow, ask, and block.
- [x] T1.1 -- Expand canonical `SecurityEvent` to typed optional roots.
- [x] T1.2 -- Implement `PolicySubject` for canonical `SecurityEvent`.
- [x] T1.3 -- Make missing-root CEL field access evaluate as non-match.
- [x] T1.4 -- Add single cross-root rule tests without fan-out.
- [x] T1.5 -- Add full first-party root coverage tests for HTTP, DNS, MCP, model, file, process, credential, and snapshot.
- [x] T2.1 -- Build new `SecurityRule` compiler from `corp.rules`, `profiles.rules`, and provider convenience settings.
- [x] T2.2 -- Compile CEL once per authored rule.
- [x] T2.3 -- Preserve `rule_id`, mandatory `rule_name`, provider id, action, detection_level, plugin, source, namespace, and priority.
- [x] T2.4 -- Remove provider-rule dependence on `ProviderRuleCallback`.
- [x] T2.5 -- Stop compiling provider rules into per-callback child rules.
- [x] T2.6 -- Delete old provider-rule compiler instead of keeping a compatibility shim.
- [x] T3.1 -- Route plugin execution through `SecurityEventEngine`.
- [x] T3.2 -- Make credential broker postprocess rule use the new rule metadata.
- [x] T3.3 -- Add plugin failure semantics and tests.
- [x] T4.1 -- Add rule-match ledger row output for every matched rule, including `rule_id`, `rule_action`, `detection_level`, rule snapshot, matched event payload, trace id, and the triggering event id.
- [x] T4.2 -- Wire enforcement materialization for `allow`, `ask`, and `block` to consume the same ledgered rule result.
- [x] T4.3 -- Add ask pending-resolution records.
- [x] T4.4 -- Block boundary materialization while ask is unresolved.
- [x] T4.5 -- Add ask approval/denial resolution path.
- [x] T4.6 -- Add OTEL rule labels: `rule_id`, `rule_name`, `rule_action`, `rule_detection_level`, `provider`.
- [x] T4.7 -- Prove rule ledger snapshots do not include raw credential observation values.
- [x] T4.8 -- Add `/security/{id}/latest` exposing the full DB-backed `SecurityRuleEvent` rows.
- [x] T4.9 -- Add `/security/{id}/info` exposing DB-regenerated security rule counters.
- [x] T4.10 -- Add strict 12-lower-hex event id contract for rule ledger rows and generated primary event ids.
- [x] T5.1 -- Wire HTTP request/response producers into unified engine.
- [x] T5.2 -- Wire DNS producers into unified engine.
- [x] T5.3 -- Wire full MCP producer surface into unified engine.
- [x] T5.4 -- Wire model request/response/tool-call/tool-response into unified engine.
- [x] T5.5 -- Wire file import/export/read/create/write/delete producers into unified engine.
- [x] T5.5a -- Wire file monitor create/write/delete and snapshot-revert restored/deleted producers into unified engine.
- [x] T5.5b -- Wire explicit file import/export/read producer boundaries when those event emitters land.
- [x] T5.6 -- Wire process exec/complete/audit producers into unified engine.
- [x] T5.7 -- Wire credential observed/substitution and snapshot producers into unified engine.
- [x] T5.8 -- Add shared primary event id handoff: `emit_security_write` allocates one 12-lower-hex id, persists it on the primary event row, and returns it for rule ledger reuse.
- [x] T5.9 -- Add DB join proof that a primary runtime event and its `security_rule_events` row share the same `event_id`.
- [x] T5.10 -- Add DB-facing rule bridge: evaluate one `SecurityRuleSet` against one canonical `SecurityEvent` and emit every matched rule row with the caller-provided primary event id.
- [x] T5.11 -- Prove matched-rule bridge emits all matches and emits zero rows for non-matches.
- [x] T5.12 -- Compile provider/default security rules into `MergedPolicies.security_rules` with source-aware priorities and a reloadable runtime handle.
- [x] T5.13 -- Wire MITM HTTP/model telemetry to emit matching rule ledger rows using the primary event id from the primary logger row.
- [x] T6.1 -- Refactor Sigma import into typed `SecurityRule`.
- [x] T6.2 -- Require/derive valid Sigma rule `name`, `detection_level`, and `match`.
- [x] T6.3 -- Validate Sigma CEL against canonical `SecurityEvent` roots.
- [x] T6.4 -- Remove separate Sigma callback/string registries.
- [x] T6.5 -- Add stale Sigma event-string rejection tests.
- [x] T7.1 -- Remove old provider `on`.
- [x] T7.2 -- Remove old provider `if`.
- [x] T7.3 -- Remove old provider `decision`.
- [x] T7.4 -- Remove old provider `actions`.
- [x] T7.5 -- Remove stale credential block guidance from sprint docs.
- [x] T7.6 -- Add burn guard so adding a new first-party event root requires rule/CEL coverage.
- [x] T8.1 -- Run focused schema tests.
- [x] T8.2 -- Run focused CEL/security-event tests.
- [x] T8.3 -- Run focused plugin/action tests.
- [x] T8.4 -- Run focused ask resolution tests.
- [x] T8.5 -- Run focused Sigma import/refactor tests.
- [x] T8.6 -- Run focused runtime integration tests.
- [x] T8.6a -- Run focused MITM telemetry hook runtime tests for HTTP/model rule-ledger joins.
- [x] T8.6b -- Run focused DNS telemetry/process tests for canonical DNS event conversion and DNS rule-ledger joins.
- [x] T8.6c -- Run focused MCP frame tests for tool calls, provider defaults, and notification rule-ledger joins.
- [x] T8.6d -- Run focused file monitor and file-tool tests for file primary-row/rule-ledger joins.
- [x] T8.6e -- Run focused process tests for exec start, exec completion, audit, and shared exec event ids.
- [x] T8.6f -- Run focused credential and snapshot tests for substitution/snapshot primary-row/rule-ledger joins.
- [x] T8.6g -- Run focused builtin MCP compile check with security rules loaded for file-tool producers.
- [x] T8.6h -- Prove snapshot revert file events emit through the async security engine from the async builtin MCP path.
- [x] T8.7 -- Query session DB for detection/enforcement/plugin labels.
- [x] T8.8 -- Run `cargo fmt --check -p capsem-core`.
- [x] T8.9 -- Run focused `policy_config` and `security_engine` capsem-core suites.
- [x] T8.10 -- Run broader release gate selected by the active branch state.
- [x] T8.11 -- End-of-sprint docs gate: add one admin rule reference page with all fields, implicit defaults, and parser-tested examples.
- [x] T8.12 -- End-of-sprint docs cleanup: delete/replace old public `policy.*` / `on` / `if` / `decision` rule syntax pages so admins see one contract.
- [ ] T9.1 -- Reconcile `sprints/perf-observability-network-lab/credential-broker-rule-memo.md` into the current rule contract.
- [ ] T9.2 -- Add the full Agent Vault-derived credential provider catalog or explicitly reject each omitted provider with rationale.
- [ ] T9.3 -- Add parser/compile tests for accepted credential broker plugin
  config and broker-owned filtering. Rules must reject `plugin = "credential_broker"`.
- [ ] T9.4 -- Add runtime substitution tests for accepted credential rendering types beyond the current API-key/header and query-reference path.
- [ ] T9.5 -- Remove invalid memo-only `credential.name` predicates from any proposed rule before implementation; raw credential names remain broker-private.

## Notes

- Decision: One authored rule is never split by callback family.
- Decision: Boundary hooks exist only to create/pass `SecurityEvent`; they do
  not own rule semantics.
- Decision: `reason` is optional.
- Decision: `name` is mandatory for every rule because logs, DB rows, and OTEL
  need stable rule identity.
- Decision: Detection is optional `detection_level` metadata, not an action.
- Decision: `detection_level` accepts `info` as shorthand but canonicalizes to
  `informational`; old `level` authoring is rejected.
- Decision: Credential broker is `postprocess`; PII is `preprocess` because it
  must enrich/redact before risk evaluation.
- Decision: Raw HTTP authorization headers are plugin-private and must not
  become first-party CEL fields.
- Decision: `file.content` remains available for on-disk file/PII scanning, but
  broker defaults must not treat it as an internet credential parser.
- Decision: File security events use canonical verbs
  `import`, `export`, `read`, `create`, `write`, and `delete`; each verb exposes
  `path`, `name`, `ext`, `mime_type`, and `content`.
- Decision: AI/provider blocks are profile/corp policy. Provider-scoped rules
  are convenience/default authoring only and normalize into the `profiles`
  namespace before runtime.
- Decision: MCP convenience authoring may be added later, but it must normalize
  into `profiles.rules` or `corp.rules`; it is not a separate rule home.
- Decision: Public rule docs cleanup happens at the end of the sprint, after
  the runtime/API shape is stable.
- Decision: Provider defaults/user/corp rules share the same schema.
- Decision: `ask` includes pending/resolution state and must block
  materialization until resolved.
- Decision: Sigma detections use the same `SecurityRule` contract and cannot
  keep a separate callback/string registry.
- Completed: Added `SecurityRuleProfile` T0 contract parser with enum-backed
  `SecurityRuleAction`, `DetectionLevel`, source-based priority defaults, and a
  real fixture at `sprints/security-event-rule-spine/fixtures/rules.toml`.
- Completed: Added first-principle `corp.rules` and `profiles.rules` support;
  provider convenience rules compile with runtime namespace `profiles`.
- Completed: `CompiledSecurityRule` now carries a compiled condition object, so
  runtime matching does not reparse rule CEL strings.
- Completed: Added `SecurityRuleSet`, a deterministic compiled-rule rail that
  evaluates one canonical `SecurityEvent` without callback fan-out and exposes
  detection, enforcement, preprocess, and postprocess rule views.
- Completed: Rebuilt built-in provider defaults in
  `crates/capsem-core/src/net/policy_config/default_provider_rules.toml` using
  the new rule contract. Provider defaults now cover OpenAI, Anthropic/Claude,
  Google/Gemini, and Ollama without `on`, `if`, `decision`, `actions`,
  `file.ingress`, or `credential.name`.
- Gap: The Agent Vault-derived credential broker memo is not fully
  implemented. The current provider defaults cover OpenAI, Anthropic/Claude,
  Google/Gemini, and Ollama. They do not yet cover the memo's 22-provider
  catalog (`aws-s3`, `cloudflare`, `datadog`, `github`, `jira`, `linear`,
  `notion`, `npm`, `npmgh`, `pagerduty`, `postmark`, `resend`, `sendgrid`,
  `sentry`, `shopify`, `slack`, `stripe`, `supabase`, `twilio`, `vercel`,
  plus the already-covered OpenAI/Anthropic). The memo also contains
  `credential.name` predicates that are intentionally not valid CEL roots.
- Completed: Burned the old provider-owned callback compiler. The remaining
  `ProviderRuleProfile` type is a settings adapter over `SecurityRuleProfile`;
  it no longer defines provider callbacks and no longer emits generated
  per-callback `PolicyConfig` rules.
- Completed: Priority validation now allows stronger corp priorities such as
  `-1000` and later user/plugin priorities such as `1000`, while rejecting
  negative non-corp priorities and any explicit priority outside
  `[-1000, 1000]`.
- Completed: Old callback-shaped provider fields `on`, `if`, `decision`, and
  `actions` are rejected by the new contract parser.
- Revised: `SecurityEventEngine` now evaluates a `SecurityRuleSet` against one
  canonical `SecurityEvent`; enabled plugins run by plugin-owned stage and
  filtering, not by `plugin = "..."` on matched rules.
- Revised: Plugin execution is staged by plugin metadata: pre-decision plugins
  run before CEL enforcement and post-decision plugins run after enforcement
  selection. Configured missing plugins fail closed before emission.
- Revised: `credential_broker` is registered as a plugin and brokers
  `SecurityEvent` credential observations without exposing raw credentials to
  CEL or using matched rule metadata.
- Revised: `credential.reference` is not a first-party CEL field alias for
  the credential reference root, matching the new rule authoring language.
- Completed: `emit_matching_security_rules_with_decision` and its blocking
  twin evaluate a `SecurityRuleSet` once, write every matched
  `security_rule_events` row, and return the enforcement decision from the
  same matched rule set. The older count-only helpers delegate to this path.
- Completed: `SecurityEnforcementDecision` / `SecurityEnforcementAction`
  represent typed `allow`, `ask`, and `block` decisions with the matching
  `rule_id`, `rule_name`, and reason. No-match and non-enforcement-only matches
  default to `allow`.
- Completed: HTTP upstream materialization has a typed enforcement guard:
  `materialize_http_request_for_upstream_after_enforcement` only materializes
  on `allow`; `ask` and `block` fail before upstream materialization.
- Completed: Added append-only `security_ask_events` rows through the logger
  sink. Rows carry strict 12-hex `ask_id`, triggering 12-hex `event_id`,
  `event_type`, `rule_id`, `rule_name`, enum-backed status
  (`pending`, `approved`, `denied`), rule snapshot, matched event payload,
  optional resolver/reason, and trace id.
- Completed: Typed ask decisions now emit pending ask rows from the same
  evaluation that writes `security_rule_events`, and return the generated
  `ask_id` on `SecurityEnforcementDecision`.
- Completed: Ask resolution is append-only. `approved` resolves the ask into
  an `allow` decision; `denied` resolves it into `block`; `pending` continues
  to block materialization.
- Completed: `security.ask` is registered as an internal runtime security
  event type so ask lifecycle rows remain on the typed event identity rail.
- Completed: Rule match tracing now carries stable low-cardinality labels from
  the matched rule snapshot: `rule_id`, `rule_name`, `rule_action`,
  `rule_detection_level`, and `provider`, plus the triggering event id/type and
  trace id.
- Completed: Sprint docs no longer contain separate credential-block
  authoring guidance. Credential handling is documented as plugin-private
  broker work selected by normal safe-context rules, with raw credential
  material outside first-party CEL fields.
- Completed: `SECURITY_EVENT_CEL_ROOTS` is now the canonical first-party root
  registry used by rule validation. The `security_event_cel_exposes_all_first_party_roots`
  test asserts its coverage set exactly matches that registry, so adding a new
  root requires a matching CEL proof.
- Completed: Added `security_rule_events` as the forensic ledger for rule
  matches. Rows carry the 12-hex triggering `event_id`, `event_type`,
  `rule_id`, enum-backed `rule_action`, non-null enum-backed
  `detection_level` (`none` when absent), the canonical rule snapshot, the
  normalized `SecurityEvent` payload that matched, and optional `trace_id`.
- Completed: New primary event rows generated by the logger now receive a
  12-lower-hex event id. The remaining runtime wiring work is to pass that same
  id into the rule ledger emitter for each producer, rather than minting a
  disconnected id at the rule-match boundary.
- Completed: Added DB-backed security endpoints:
  `GET /security/{id}/latest` returns `Vec<SecurityRuleEvent>` directly, and
  `GET /security/{id}/info` returns `SecurityRuleStats` directly. These are
  regenerated from `session.db`; there is no second detection/enforcement path.
- Completed: Primary logger event structs now carry optional `event_id`.
  `WriteOp::ensure_event_id()` assigns a 12-lower-hex id before the event
  crosses the security emitter boundary, and the writer preserves supplied ids
  while still generating one for direct logger calls.
- Completed: Runtime producers touched by this slice now construct logger
  events with `event_id: None` and rely on the security emitter/writer to own
  id allocation. This includes MITM HTTP/model/MCP telemetry, DNS telemetry,
  file monitor/tools, process exec/audit/snapshot, built-in tool HTTP, and
  credential substitution events.
- Completed: Added `emit_matching_security_rules` and
  `emit_matching_security_rules_blocking`. Producers now have the public API
  needed to persist the primary event, evaluate the canonical `SecurityEvent`
  against a `SecurityRuleSet`, and write all matched ledger rows with the same
  event id.
- Completed: `MergedPolicies` now carries a compiled `SecurityRuleSet` from
  built-in provider defaults plus user and corp provider rules. The
  capsem-process runtime keeps that ruleset in a reloadable shared handle so
  MITM hooks do not compile or parse rules on request paths.
- Completed: MITM telemetry now writes the primary HTTP/model logger row
  through `emit_security_write`, converts the logger event into the canonical
  `SecurityEvent`, evaluates the active `SecurityRuleSet`, and writes matched
  `security_rule_events` rows with the same 12-lower-hex `event_id`.
- Completed: DNS vsock telemetry now writes the primary `dns_events` row
  through the same security emitter, converts the `DnsEvent` into canonical
  `SecurityEvent.dns`, evaluates the active `SecurityRuleSet`, and writes
  matched `security_rule_events` rows with the same primary event id.
- Completed: MCP framed telemetry now carries the runtime `SecurityRuleSet`
  on `McpEndpointState`, writes `mcp_calls` through the security emitter,
  converts `McpCall` rows into canonical `SecurityEvent.mcp`, and writes
  matched `security_rule_events` rows with the same primary event id. This now
  includes notification-only MCP frames, which are logged without fabricating a
  JSON-RPC response to the guest.
- Completed: Explicit file boundaries now use the unified security-event
  emitter. Process-backed `read_file` and `write_file` jobs record
  `file.read` and `file.write` roots after guest acknowledgement, while
  service host-workspace upload/download requests send a typed
  `LogFileBoundary` IPC command to capsem-process so `file.import` and
  `file.export` rows are written by the process-owned DB writer and matched
  against the active `SecurityRuleSet`. No service-side DB writer or fallback
  logger was added.
- Completed: Sigma detection YAML now imports through
  `SecurityRuleProfile::parse_sigma_yaml`, compiles into typed
  `SecurityRule` entries, validates generated `match` expressions against the
  canonical `SecurityEvent` CEL roots, and merges referenced `rule_files.sigma`
  files into `MergedPolicies.security_rules`.
- Completed: Old callback-shaped Sigma import tests were removed from
  `validate_imported_policy_rule_json`; stale Sigma fields such as
  `request.host` are now rejected because they are not first-party
  `SecurityEvent` roots.
- Verification: `cargo test -p capsem-core --lib security_rule_profile -- --nocapture`
  passed with 18 focused tests, including Sigma import, Sigma evaluation over
  `SecurityEvent`, and stale Sigma field rejection.
- Completed: Canonical `SecurityEvent` now carries typed optional CEL roots for
  HTTP, DNS, MCP, model, file, process, credential, and snapshot.
- Completed: `SecurityEvent` implements `PolicySubject`, and cross-root OR
  expressions evaluate as one rule without callback fan-out.
- Completed: Missing roots evaluate as non-matches.
- Verification: `cargo test -p capsem-core --lib security_event_cel -- --nocapture`
  passed with 4 focused tests.
- Verification: `cargo test -p capsem-core --lib policy_config -- --nocapture`
  passed with 450 tests.
- Verification: `cargo test -p capsem-core --lib 'net::policy_config::loader' -- --nocapture`
  passed with 24 focused loader tests, including relative Sigma rule-file
  loading.
- Verification: `cargo test -p capsem-core --lib emit_explicit_file_security_events_map_import_export_and_read_roots -- --nocapture`
  passed, proving `file.import`, `file.export`, and `file.read` rule roots
  write primary `fs_events` rows plus matched `security_rule_events` payloads.
- Verification: `cargo test -p capsem-proto log_file_boundary -- --nocapture`
  passed, proving the typed `FileBoundaryAction` IPC command/result roundtrip.
- Verification: `cargo test -p capsem-process classify_log_file_boundary -- --nocapture`
  passed, proving the process IPC classifier treats file-boundary logging as a
  job instead of an unexpected command.
- Verification: `cargo check -p capsem-service -p capsem-process -p capsem-proto`
  passed after wiring service upload/download through process-owned boundary
  logging.
- Verification: `uv run pytest tests/capsem-security/test_detection_yaml.py -q`
  passed with 1 Python parser compatibility test for `detection.yaml`.
- Verification: `cargo test -p capsem-core --lib security_engine -- --nocapture`
  passed with 39 tests, including typed `SecurityRuleSet` plugin execution,
  preprocess/postprocess stage re-evaluation, missing-plugin fail-closed
  semantics, credential-broker postprocess execution from rule metadata,
  typed enforcement decisions derived from the same evaluation as the rule
  ledger, default allow for non-enforcement matches, HTTP materialization
  refusal for unresolved `ask`/`block`, ask pending row emission, ask
  approval/denial resolution semantics, stable OTEL-style rule labels,
  12-hex `SecurityEventId` generation/parsing, forensic rule ledger emission,
  primary-event/rule-ledger event id join, all matched rule ledger emission,
  DB-regenerated detection/enforcement/plugin labels plus ask lifecycle rows,
  non-match zero-row behavior, file producer helpers, process exec/complete
  shared ids, credential substitution, and snapshot joins.
- Verification: `cargo test -p capsem-core --lib telemetry_hook -- --nocapture`
  passed with 13 focused tests, including HTTP and model telemetry producing
  DB-joined `security_rule_events` rows with the primary logger event id.
- Verification: `cargo test -p capsem-core --lib fs_monitor -- --nocapture`
  passed with 17 focused tests, including file monitor primary-row/rule-ledger
  joins and credential broker reference persistence for `.env` captures.
- Verification: `cargo test -p capsem-core mcp::file_tools -- --nocapture`
  passed with 45 focused tests after snapshot revert was split into a sync
  revert/result helper plus async file security emission proof. This includes
  `revert_file_security_event_emits_from_async_runtime`, which writes a real
  `fs_events` row from inside Tokio through
  `emit_file_security_write_and_rules().await`.
- Verification: `cargo test -p capsem-core --lib dns::telemetry -- --nocapture`
  passed with 8 focused tests, including canonical DNS security-event
  conversion.
- Verification: `cargo test -p capsem-process vsock -- --nocapture` passed
  with 17 tests, including DNS primary-row/rule-ledger event id joins through
  the process-side emitter and the no-stall exec completion path with the new
  rules handle.
- Verification: `cargo test -p capsem-core --lib mcp_frame -- --nocapture`
  passed with 50 focused tests, including MCP tool-call and notification
  primary-row/rule-ledger event id joins, plus built-in provider MCP defaults
  logging through `security_rule_events` instead of generated old
  `policy.mcp.*` provider callbacks.
- Verification: `cargo test -p capsem-logger -- --nocapture` passed with 116
  unit tests plus 126 roundtrip tests, including strict
  `security_rule_events` and `security_ask_events` schema checks,
  DB-regenerated stats, ask lifecycle row roundtrip, primary event id
  generation, and supplied primary event id preservation.
- Verification: `cargo check -p capsem-logger -p capsem-core -p capsem-process -p capsem-service -p capsem-mcp-builtin`
  passed after adding shared event IDs to logger structs and runtime producers.
- Verification: `cargo check -p capsem-mcp-builtin` passed after loading
  `MergedPolicies.security_rules` in the builtin MCP subprocess.
- Verification: `cargo test -p capsem-mcp-builtin -- --nocapture` passed.
- Verification: `cargo build -p capsem-mcp-builtin -p capsem-mcp-aggregator`
  passed, rebuilding the actual `target/debug` MCP binaries used by VM E2E.
- Verification: `CAPSEM_REQUIRE_ARTIFACTS=1 uv run python -m pytest tests/capsem-e2e/test_e2e_lifecycle.py::TestDoctor::test_doctor_passes -v --tb=short -s`
  passed in 37.32s after rebuilding the actual host MCP binaries. Prior failed
  artifact showed `capsem-mcp-builtin` panicking on `DbWriter::write_blocking`
  inside Tokio during `snapshots_revert`; the fixed path emits file events
  asynchronously after releasing the snapshot lock.
- Verification: `CAPSEM_REQUIRE_ARTIFACTS=1 uv run python -m pytest tests/capsem-e2e/test_framed_mcp_mitm.py::test_framed_guest_mcp_tools_call_and_session_db_rows tests/capsem-e2e/test_framed_mcp_mitm.py::test_framed_guest_mcp_invalid_json_notifications_and_string_ids -v --tb=short`
  passed after updating the MCP session DB expectations for notification rows
  with `request_id = NULL`.
- Verification: `CAPSEM_REQUIRE_ARTIFACTS=1 uv run python -m pytest tests/capsem-serial/test_lifecycle_benchmark.py::test_fork_benchmark -v --tb=short -s`
  passed with EROFS/LZ4HC fork benchmark numbers: fork min/mean/max
  26/28/29 ms, image min/mean/max 12.6/12.7/12.7 MB under the updated 13 MB
  gate, boot provision min/mean/max 922/925/929 ms, boot ready min/mean/max
  10/12/12 ms. Benchmark JSON:
  `benchmarks/fork/data_1.0.1780610732.json`.
- Verification: `just test` passed end-to-end. Highlights from the release
  gate: Python main suite `1329 passed, 69 skipped` with 91.15% coverage;
  build-chain suite `27 passed`; injection suite `5 passed, 0 failed`;
  integration ledger `40 passed, 0 failed` with guest doctor subset
  `94 passed, 2 skipped`; install E2E container suite `30 passed, 26 skipped`;
  Linux arm64 `.deb` built and validated, with boot test intentionally skipped
  because local KVM/cross-arch boot is not available. Dev Tauri updater key
  mismatch warning remains expected dev-signing noise, not a runtime test
  failure.
- Verification: `just test` generated fresh benchmark artifacts:
  `benchmarks/lifecycle/data_1.0.1780610732.json` with lifecycle total
  min/mean/max 1052.7/1075.7/1113.8 ms, provision 971.9/993.2/1030.9 ms,
  exec-ready 10.9/11.8/12.9 ms, exec 9.8/10.2/10.6 ms, delete
  59.3/60.4/61.1 ms; `benchmarks/capsem-bench/data_1.0.1780610732_arm64.json`
  with disk sequential write/read 1777.7/4326.0 MB/s, random write/read
  7407.0/52983.3 IOPS, rootfs sequential read 3198.6 MB/s, rootfs random read
  32775.1 IOPS, HTTP 50/50 success at 65.7 req/s with p50/p95/p99
  59.0/203.0/207.5 ms, and throughput 22.54 MB/s.
- Verification: Rebuilt the public policy documentation around the
  `SecurityEvent` rule contract in `docs/src/content/docs/security/policy.md`.
  The page now documents rule homes, fields, defaults, actions, priority
  discipline, CEL roots, parser-tested TOML examples, Sigma YAML import, and
  DB ledger expectations.
- Verification: Burned stale public rule syntax from the MCP gateway, MITM
  proxy, settings schema/settings flow, network isolation, session telemetry,
  and just-recipe docs. Follow-up scan
  `rg -n 'policy\.http|policy\.mcp|policy\.dns|\bon\s*=|\bif\s*=|decision\s*=|actions\s*=|policy_action =|decision = "block"|request\.host|tool\.name|mcp\.request|mcp\.response|dns\.response' docs/src/content/docs -g '*.md' -g '*.mdx'`
  only returns the intentional "do not use old callback-local roots" warning
  in the new policy page.
- Verification: `cargo test -p capsem-core --lib security_rule_profile -- --nocapture`
  passed 18/18 after the docs pass, proving the documented fixtures and old
  syntax rejection still compile through the Rust parser.
- Verification: `pnpm -C docs install --frozen-lockfile && pnpm -C docs run build`
  passed after installing missing docs dependencies; Astro/Starlight built
  44 pages.
- Verification: `cargo check -p capsem-core` passed after adding the Sigma
  YAML parser dependency and profile merge path.
- Verification: `cargo check -p capsem-core` passed after typed rule-plugin
  execution was added.
- Verification: `cargo fmt --check -p capsem-logger -p capsem-core -p capsem-process -p capsem-service -p capsem-mcp-builtin`
  passed.
- Verification: `cargo fmt --check -p capsem-core` and `git diff --check`
  passed after the Sigma refactor.

## Coverage Ledger

- Unit/contract: T0 schema, priority, name, detection_level, enum action, plugin
  requirement, and old-field rejection covered by `security_rule_profile`
  tests.
- Functional: Planned single-rule cross-root CEL evaluation through
  `SecurityEvent`; T1 focused tests now cover HTTP/model/file OR,
  DNS missing-root false behavior, safe credential reference fields, and all
  first-party root paths.
- Adversarial: Planned malformed CEL, missing root, invalid plugin, bad name,
  bad priority, and stale old-shape rejection tests.
- Integration: Focused HTTP/DNS/MCP/model/file/process/credential/snapshot
  producer tests now cover primary rows plus matched `security_rule_events`
  joins. Explicit file import/export/read boundaries are covered through the
  typed service-to-process IPC rail and the process-owned security emitter.
- Ask: Planned pending, approval, denial, timeout/cancel, and audit row tests.
- Sigma: `SecurityRuleProfile::parse_sigma_yaml` imports parser-compatible
  Sigma YAML into the same `SecurityRule` parser, validator, and event-root
  registry. Tests cover fixture import, runtime evaluation, relative
  `rule_files.sigma` loading, stale callback-field rejection, and Python parser
  compatibility.
- Session DB: `security_rule_events` now stores every rule match as the
  forensic ledger row with rule snapshot and matched event payload. MITM
  HTTP/model telemetry, DNS telemetry, MCP telemetry, file monitor/tool
  telemetry, explicit file import/export/read/write boundaries, process
  exec/audit/complete, credential substitution, and snapshots are wired and
  tested with the primary event id.
- E2E/VM: Focused doctor now passes after rebuilding the actual host MCP
  binaries. Lesson captured: `cargo test` proves the Rust library and test
  harness, but VM E2E uses `target/debug/capsem-mcp-builtin`, so host MCP
  binary changes require an explicit binary build before doctor proof.
- Telemetry: Planned OTEL rule-label and no-raw-secret checks.
- Performance: Fork benchmark passes with explicit EROFS/LZ4HC numbers:
  fork 26/28/29 ms, image 12.6/12.7/12.7 MB, boot provision 922/925/929 ms,
  boot ready 10/12/12 ms.
- Performance: Full `just test` also produced lifecycle min/mean/max
  1052.7/1075.7/1113.8 ms and in-VM capsem-bench numbers for disk, rootfs,
  startup, HTTP, throughput, and snapshot operations in
  `benchmarks/capsem-bench/data_1.0.1780610732_arm64.json`.
- Docs: Admin-facing rule reference now matches the implemented
  `SecurityRuleProfile` contract, and stale public examples for old
  `policy.*` / `on` / `if` / `decision` authoring have been removed or
  replaced with the security-event ledger model.
- Missing/deferred: Full credential broker rule memo catalog conversion is
  open as T9. The current branch proves the rule/plugin/ledger rail and a
  small provider set, but it does not yet prove every memo provider or every
  desired credential rendering type.
