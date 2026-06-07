# S06d - Core Structure And Test Boundaries

## Status

Done.

## Goal

Make the current network/security code easier to test and reason about before
the post-S06 rename and before S08b introduces first-class Network, File,
Process, Security, and Emitter engine contracts.

This is a structural hygiene sprint, not an engine-design sprint. We split large
modules and large test files inside `capsem-core` while deliberately deferring
new crate boundaries until the engine contracts are written.

## Why Now

S06c removed the second policy runtime, which simplified the authority model.
The remaining problem is shape: several files are now carrying too many
responsibilities.

- `crates/capsem-core/src/net/mitm_proxy/mod.rs` mixes config, connection
  handshake, request handling, upstream dispatch, policy dispatch, body preview,
  and telemetry glue.
- `crates/capsem-core/src/net/mitm_proxy/tests.rs` is a broad regression bucket
  covering policy hot reload, upstream failures, model policy, HTTP policy,
  body handling, telemetry, and connection behavior.
- `crates/capsem-core/src/net/dns/tests.rs` mixes DNS policy decisions, cache,
  resolver failover, telemetry, metrics, and rewrite behavior.
- `crates/capsem-core/tests/mitm_integration.rs` mixes TLS interception, plain
  HTTP, local model/Ollama-shape tests, telemetry, and policy regression tests.

If we start S08b with these files still too dense, engine extraction will be
harder to review and easier to get subtly wrong.

## Scope

- Split `mitm_proxy/mod.rs` into smaller internal modules without changing the
  public API:
  - config/shared deps;
  - connection/TLS handshake;
  - HTTP request handling;
  - upstream connection/dispatch;
  - direct telemetry/event emission helpers that remain outside hooks.
- Split `mitm_proxy/tests.rs` into behavior-focused test modules:
  - connection/metadata/WebSocket behavior;
  - HTTP Policy allow/block/ask/rewrite;
  - Policy hot reload;
  - model request/response/tool policy;
  - upstream failures;
  - telemetry and body preview behavior.
- Split `dns/tests.rs` into behavior-focused test modules:
  - Policy decisions;
  - cache semantics;
  - resolver failover/errors;
  - metrics/telemetry;
  - rewrite response behavior.
- Split `tests/mitm_integration.rs` into focused integration files if Cargo
  filtering remains straightforward:
  - plain HTTP;
  - TLS interception;
  - local model/Ollama-shaped traffic;
  - telemetry/security regressions.
- Keep module moves mechanical and behavior-preserving. Rename only where a
  local helper name becomes misleading after the move.
- Add source guards where useful so deleted V1 policy modules do not creep back.

## Non-Goals

- Do not introduce new engine crates here.
- Do not define Network Engine/File Engine/Process Engine/Security Engine
  contracts here. S08b owns that architecture.
- Do not change rule semantics, CEL/Sigma decisions, telemetry schemas, or
  `session.db` shape.
- Do not perform the broad `policy` naming collapse. That remains the
  Post-S06 cleanup milestone after this structural pass.

## Testing Matrix

- Unit/contract: moved modules compile with no public API changes; source guard
  still proves legacy V1 policy runtime is absent.
- Functional: DNS Policy tests, MITM Policy tests, hot-reload tests, and
  selected integration tests still pass after files move.
- Adversarial: fail-closed policy condition tests and upstream failure tests
  keep their current assertions.
- Integration: integration files remain intact because current filters are
  straightforward; S08b will move them only once engine boundaries are real.
- Maintainability: test names remain filterable by behavior so future sprints
  can run focused gates without searching a giant catch-all file.

## Done Means

- The large MITM and DNS files are split into coherent modules/test files with
  no behavior change.
- `cargo check -p capsem-core -p capsem-process` passes.
- `cargo test -p capsem-core --all-targets --no-run` passes.
- Focused DNS/MITM policy, hot-reload, and touched integration filters pass.
- Tracker and MASTER still show crate extraction deferred to S08b, not hidden in
  this hygiene sprint.

## Implementation Notes

- Split `crates/capsem-core/src/net/dns/tests.rs` into focused behavior modules:
  `policy_decisions`, `resolver_behavior`, `rewrite_behavior`,
  `metrics_behavior`, and `cache_behavior`.
- Split the MITM connection/metadata/FD/TLS behavior bucket into
  `tests/connection_behavior.rs`.
- Split the largest MITM policy regression buckets out of
  `crates/capsem-core/src/net/mitm_proxy/tests.rs` into
  `tests/model_policy.rs` and `tests/http_policy.rs`; the remaining harness
  still owns shared fixtures plus smaller utility/body/upstream tests.
- Extracted production MITM upstream TLS and debug override resolution into
  `crates/capsem-core/src/net/mitm_proxy/upstream.rs`.
- Extracted production hook pipeline construction into
  `crates/capsem-core/src/net/mitm_proxy/pipeline_factory.rs`.
- Extracted gzip response header classification into
  `crates/capsem-core/src/net/mitm_proxy/response.rs`.
- Left `crates/capsem-core/tests/mitm_integration.rs` intact because the
  current file remains straightforward to filter and S08b will decide the
  production engine boundary that those integration tests should follow.
- Added a source guard proving the deleted `NetworkPolicy`/V1 MITM hook files
  stay deleted and runtime call sites do not import the old runtime.

## Verification

- `cargo fmt --package capsem-core`
- `cargo check -p capsem-core -p capsem-process`
- `cargo test -p capsem-core runtime_call_sites_do_not_import_legacy_network_policy_runtime --lib`
- `cargo test -p capsem-core net::mitm_proxy::tests::connection_behavior --lib`
- `cargo test -p capsem-core net::mitm_proxy::tests::response_uses_gzip_content_encoding_accepts_token_lists_case_insensitively --lib`
- `cargo test -p capsem-core net::mitm_proxy::tests::upstream_connect_target_honors_debug_test_override --lib`
- `cargo test -p capsem-core policy_model_ --lib`
- `cargo test -p capsem-core policy_http_ --lib`
- `cargo test -p capsem-core policy_hot_reload --lib`
- `cargo test -p capsem-core net::dns:: --lib`
- `cargo test -p capsem-core --all-targets --no-run`
- `git diff --check`
