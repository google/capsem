# Sprint: 1.3 Hardening Debug Pass

## Tasks

- [x] T0 Debug evidence route/status inventory
- [x] T1 Aggregator fail-loud
- [x] T2 Unknown-domain AI sniffing
- [x] T3 Broker reuse/replay evidence
- [x] T4 Raw VSOCK boundary
- [x] Changelog/docs
- [x] Final verification gate
- [x] Commit and push

## Notes

- Start from clean tree on `release/1.3-cleanup-pr-v2`.
- Do not kill or purge the current AGY evidence VM.
- DNS-local rate limiting is explicitly out of scope; issue #69 owns general
  security-rail rate limiting.
- Skill manager is out of scope; issue #70 owns that epic.
- T1: `capsem-process` no longer returns an empty MCP aggregator stub when
  `capsem-mcp-aggregator` is missing. The resolver supports both installed
  sibling binaries and cargo-test `target/debug/deps -> target/debug` layout,
  but otherwise fails loud with the missing component named.
- T2: Unknown/private model gateways now get bounded JSON body-shape sniffing.
  Only known-length JSON `POST`/`PUT`/`PATCH` bodies up to `AI_BODY_PREVIEW`
  are collected; oversized/chunked/irrelevant bodies stay on the normal HTTP
  path. Promoted events carry both `http.host` and `model.provider`, and the
  telemetry hook receives an explicit `model_traffic` bit so neutral private
  paths can emit `model_calls` without broadening known-provider non-model
  endpoints.
- T3: Credential broker runtime inventory now reports `replay_available` per
  credential ref. This is derived by resolving the broker reference from the
  broker store/keychain, not by trusting session DB substitution rows. The AGY
  loop can now distinguish "observed in ledger" from "actually reusable for a
  later VM/profile/fork".
- T4: Raw host VSOCK services now live in the typed `HostVsockService`
  registry in `capsem-proto`. `boot_vm` registers exactly
  `host_vsock_ports()`, `capsem-process` dispatches through
  `HostVsockService::from_port`, retired raw MCP port `5003` stays closed, and
  guest TCP service ports such as `11434` remain MITM redirect traffic rather
  than raw VSOCK listeners.
- T0: `capsem debug` support bundles now include
  `system/runtime-boundary.json` with the first-party VSOCK service list,
  explicitly closed raw ports, and route-backed debug/status surfaces.

## Coverage Ledger

- Unit/contract: `cargo test -p capsem-process mcp_aggregator -- --nocapture`
  proves missing aggregator is an error and cargo-test dev layout resolves a
  real sibling binary.
- Unit/contract: `cargo test -p capsem-core provider_detection -- --nocapture`
  and `cargo test -p capsem-core unknown_model_body_sniffing --lib --
  --nocapture`.
- Functional/integration: `cargo test -p capsem-core --test mitm_integration
  mitm_proxy_plain_http_unknown_openai_shape_emits_model_call -- --nocapture`
  proves an unknown private OpenAI-shaped HTTP endpoint forwards the original
  request body and emits a first-party `ModelCall`.
- Unit/contract: `cargo test -p capsem-core
  replay_availability_requires_resolvable_broker_secret -- --nocapture`.
- Functional: `cargo test -p capsem-service
  credential_broker_plugin_runtime_reports_session_db_substitutions --
  --nocapture` proves DB-only evidence reports `replay_available=false`.
- Unit/contract: `cargo test -p capsem-proto
  host_vsock_registry_is_the_only_boot_listener_contract -- --nocapture` proves
  the raw VSOCK listener contract is a typed registry and rejects retired/raw
  TCP ports.
- Unit/contract: `cargo test -p capsem-process classify_ -- --nocapture`
  proves process-side VSOCK classification includes control, terminal, MITM,
  lifecycle, exec, audit, and DNS.
- Functional/observability: `cargo test -p capsem
  bundle_includes_runtime_boundary_debug_contract -- --nocapture` and
  `cargo test -p capsem support_bundle -- --nocapture` prove `capsem debug`
  carries the new boundary/debug artifact without leaking gateway tokens.
- Adversarial: missing aggregator binary; oversized/irrelevant model bodies;
  DB-only credential refs; retired VSOCK port `5003`; guest TCP port `11434`
  as raw VSOCK.
- E2E/VM or integration: Unknown private OpenAI-shaped HTTP endpoint covered by
  MITM integration test; final AGY/manual VM loop remains pending user action
  after this gate.
- Telemetry/observability: structured logs now identify missing aggregator,
  unknown model body promotion, broker replay availability, unknown VSOCK
  rejection, and support-bundle boundary facts.
- Performance: bounded body sniffing only collects known-length JSON bodies up
  to `AI_BODY_PREVIEW`; no benchmark required for this diagnostic hardening
  slice.
- Missing/deferred: full host-side OAuth replay adapter is not implemented in
  this slice; the broker now exposes truthful `replay_available` evidence so
  that adapter can be tested without pretending DB rows are enough.

## Final Gate

- `cargo fmt --check`
- `cargo test -p capsem-process mcp_aggregator -- --nocapture`
- `cargo test -p capsem-core provider_detection -- --nocapture`
- `cargo test -p capsem-core unknown_model_body_sniffing --lib -- --nocapture`
- `cargo test -p capsem-core telemetry_hook -- --nocapture`
- `cargo test -p capsem-core --test mitm_integration mitm_proxy_plain_http_unknown_openai_shape_emits_model_call -- --nocapture`
- `cargo test -p capsem-core replay_availability_requires_resolvable_broker_secret -- --nocapture`
- `cargo test -p capsem-service credential_broker_plugin_runtime_reports_session_db_substitutions -- --nocapture`
- `cargo test -p capsem-proto host_vsock_registry_is_the_only_boot_listener_contract -- --nocapture`
- `cargo test -p capsem-process classify_ -- --nocapture`
- `cargo test -p capsem support_bundle -- --nocapture`
- `git diff --check`
