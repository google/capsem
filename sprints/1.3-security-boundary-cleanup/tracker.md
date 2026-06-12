# Sprint: 1.3 Security Boundary Cleanup

## Status

In progress. No implementation is accepted until RED tests prove the boundary
failure first.

## Tasks

- [x] Capture sprint boundary and end posture.
- [x] RED: security-engine contract proves plugins receive a `SecurityEvent`
  and emit/return a `SecurityEvent`; no stage gets network/logger side-channel
  objects.
- [x] RED: network header formatter cannot create credential refs or
  provider-sensitive redaction.
- [x] RED: security engine logging-plugin sanitizes raw credential-bearing
  events before logger/storage materialization.
- [x] Implement explicit preprocess / postprocess / logging plugin stage
  ordering without splitting one plugin across unrelated responsibilities.
- [x] Define explicit plugin object contracts: base metadata plus preprocess,
  postprocess, and logging stages, all `SecurityEvent -> SecurityEvent`.
- [x] Extend the profile/corp plugin policy and route-visible plugin catalog to
  cover all three plugin stages explicitly: `credential_broker` is
  preprocess, `dummy_post_allow` is postprocess, and `log_sanitizer` is
  logging.
- [x] Align the core `SecurityPluginStage` enum and action benchmark matrix
  with the same three stage names: preprocess, postprocess, and logging.
- [ ] Split runtime materialization from ledger materialization.
- [x] Burn credential-sensitive logic from network formatter/intercept helpers.
- [ ] Rename/docs cleanup for touched boundaries: network engine, security
  engine, credential broker, log sanitizer.
- [ ] Update architecture docs with the explicit runtime-vs-ledger
  materialization contract.
- [ ] Update developer skills with the no-drift rule: no credential handling in
  network formatters, DB readers, frontend transforms, or one-off harnesses.
- [x] Ironbank: local OpenAI-compatible SDK credential header request reaches
  upstream while DB/log/route payloads contain no raw secret.
  - Proof: `uv run python -m pytest tests/ironbank/test_model_sdk_ledger.py -v --tb=short`
    boots a VM through service routes, drives the real OpenAI Python SDK
    against the hermetic mock server, writes the returned poem to disk, and
    asserts HTTP/model/tool/file/exec/security/substitution DB rows plus
    `/vms/{id}/info`, `/vms/{id}/status`, and `/vms/{id}/security/latest`.
- [x] Ironbank: generic HTTP credential header request reaches upstream while
  DB/log/UI route payloads contain no raw secret.
  - Proof: `uv run python -m pytest tests/ironbank/test_model_sdk_ledger.py -q`
    now replays a captured `credential:blake3:*` ref through a generic
    `/echo` HTTP request and a second OpenAI-compatible SDK call. The mock
    server proves upstream received a real authorization value instead of the
    broker ref, while `net_events.credential_ref` carries the typed ref,
    logged headers carry only `authorization: hash:*`, and
    `substitution_events` includes both `captured` and `injected`.
- [x] Ironbank: query, JSON body, form body, response token body, and model SDK
  replay get the same no-raw-ledger proof.
  - Proof: `uv run python -m pytest tests/ironbank/test_model_sdk_ledger.py -q`
    now drives a captured broker ref through header replay, query replay,
    JSON request body capture, form request body capture, OAuth token response
    capture, generic credential response capture, and a second
    OpenAI-compatible SDK model call. The mock server proves broker refs are
    not sent upstream on query replay; DB assertions prove request/response
    previews contain broker refs instead of raw credential material and
    substitution ledger rows include `captured`/`injected` sources for all
    exercised material classes.
- [ ] Add plugin latency/counter evidence for broker and sanitizer.
- [x] Update CHANGELOG.md.
- [x] Focused test gate.
- [x] Commit and push this slice before returning to broader bug hotlist.

## Invariants

- Network engine parses and routes; it does not decide, broker, redact, or
  credential-classify.
- Security engine is the only rule/plugin/decision rail.
- Plugins receive a `SecurityEvent` and emit/return a `SecurityEvent`; no
  network, logger, DB, route, or formatter object can enter the plugin contract.
- Credential broker plugin owns capture/store/inject metadata and does not own
  logging projection.
- Log sanitizer logging-plugin owns durable projection before
  logger/materializer handoff and does not care whether brokering happened.
- Upstream/runtime bytes and ledger bytes are separate materializations.
- Raw credential material must never reach session DB, structured logs, route
  JSON, or frontend stats.
- No logger-specific sanitizer fallback, compatibility rail, or formatter
  side-channel.

## Coverage Ledger

- Unit/contract:
  - `cargo test -p capsem-core header_formatter_does_not_broker_or_classify_credentials -- --nocapture`
  - `cargo test -p capsem-core security_event_log_sanitizer_logging_plugin_redacts_before_logger_emit -- --nocapture`
  - `cargo test -p capsem-core security_event_engine_ -- --nocapture`
  - `cargo test -p capsem-core security_plugin_ -- --nocapture`
  - `cargo test -p capsem-core security_event_engine_runs_enabled_plugins_by_stage -- --nocapture`
  - `cargo test -p capsem-core plugin_policy -- --nocapture`
  - `cargo test -p capsem-core parses_real_provider_defaults_as_security_rules -- --nocapture`
  - `cargo test -p capsem-core builtin_profile_contract_requires_plugins_and_visible_default_rules -- --nocapture`
  - `cargo test -p capsem-process runtime_profile_source_loads_rules_plugins_mcp_without_settings -- --nocapture`
  - `cargo test -p capsem-service profile_plugin_endpoint_matrix_dynamically_controls_enforcement_evaluation -- --nocapture`
  - `cargo test -p capsem-core builtin_dummy_plugins_block_eicar_and_cannot_be_downgraded_by_postprocess -- --nocapture`
  - `cargo test -p capsem-core credential_broker_plugin_uses_matched_security_rule_metadata -- --nocapture`
  - `cargo test -p capsem-core credential_broker_uses_ai_provider_hint_for_local_openai_compatible_headers -- --nocapture`
  - `cargo test -p capsem-core credential_broker_plugin_marks_broker_ref_for_injection_not_recapture -- --nocapture`
  - `cargo test -p capsem-core http_body_detector_finds_local_nested_credential_response -- --nocapture`
  - `cargo test -p capsem-core http_body_credential_candidate_is_limited_to_known_exchange_paths -- --nocapture`
  - `cargo test -p capsem-core http_materializer_resolves_broker_ref_only_for_upstream_copy -- --nocapture`
  - `cargo test -p capsem-core hook_writes_injected_substitution_event_for_broker_ref_replay -- --nocapture`
  - `cargo test -p capsem-core openai_non_streaming_tool_call_carries_request_trace -- --nocapture`
  - `cargo test -p capsem-core non_streaming_openai_text_survives_tool_call_response -- --nocapture`
  - `cargo test -p capsem-core` passed: 1560 unit tests, 29 MITM integration tests, 2 platform gating tests, 12 settings tests, 11 VM integration tests, doc tests ok; only existing ignored tests remained ignored.
- Functional:
  - `cargo build -p capsem-service -p capsem-process -p capsem-gateway`
    rebuilds the binaries used by the black-box harness.
- Adversarial:
  - The Ironbank fixture constructs the synthetic SDK secret at runtime so file
    import logging cannot pass because the test itself baked a raw credential
    into uploaded source.
- E2E/VM:
  - `uv run python -m pytest tests/ironbank/test_model_sdk_ledger.py -v --tb=short`
    passed.
  - `uv run python -m pytest tests/ironbank/test_model_sdk_ledger.py -q`
    passed with broker-ref replay over generic HTTP and OpenAI-compatible SDK
    traffic.
- Telemetry:
  - The Ironbank model SDK test asserts `net_events`, `model_calls`,
    `tool_calls`, `fs_events`, `exec_events`, `security_rule_events`, and
    `substitution_events` exact fields for the local OpenAI-compatible path.
  - The broker replay extension asserts injected substitution rows and that
    sanitized request headers expose neither the raw secret nor the broker ref.
  - The body/query extension asserts query injection rows and captured
    substitution rows for JSON request bodies, form request bodies, OAuth token
    response bodies, and nested credential response bodies.
- Performance: pending plugin counters/latency evidence.
  - `cargo bench -p capsem-core --bench security_actions --no-run` now
    compiles the preprocess, postprocess, and logging plugin benchmark matrix.
- Docs/skills: boundary note added to `/dev-mitm-proxy`; architecture docs still pending.
- Missing/deferred: none accepted for release blocker scope.
