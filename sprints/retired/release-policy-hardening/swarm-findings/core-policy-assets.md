# Core Policy and Assets Findings

Status: completed; transferred to T7 FD04 and owner rows in T1/T3/T6/T8/T10.
T3 hook/MCP runtime and T6 telemetry/tooling items are implemented; T1/T8/T10
downstream items remain open where listed below.

Agent: Kant (`019e1264-dba6-7ae3-b34e-20edf051132d`)
T8 hook scope audit agent: Gibbs (`019e1342-9f35-7261-a62f-953938ceb395`)

## Scope

- `capsem-core` asset manifest resolution and compatibility.
- Policy V2 hook runtime and Spec0 validation.
- Body caps, fail-closed semantics, MCP frame assumptions.
- Benchmark coverage.

## Findings

- [x] [P0] MCP notifications can bypass request policy and telemetry.
  - Paths: `crates/capsem-core/src/net/mitm_proxy/mcp_frame.rs:141`,
    `crates/capsem-core/src/net/mitm_proxy/mcp_frame.rs:156`,
    `crates/capsem-core/src/net/mitm_proxy/mcp_frame.rs:202`,
    `crates/capsem-core/src/net/mitm_proxy/mcp_frame.rs:1391`,
    `crates/capsem-core/src/net/mitm_proxy/mcp_endpoint.rs:179`.
  - Detail: request policy decision is computed, but notification frames are
    dispatched before block path; a no-id `tools/call` notification can reach
    `handle_request`.
  - Proof: adversarial `tools/call` notification test proving no aggregator
    dispatch, no argument leak, and intended denial/audit behavior.
  - Run: `cargo test -p capsem-core mcp_frame -- --nocapture`.
  - Sprint IDs: T3.5, T8.4.

- [x] [P0] Hook policy is accepted by core config but not wired to production
  dispatch.
  - Paths: `crates/capsem-core/src/net/policy_config/types.rs:338`,
    `crates/capsem-core/src/net/policy_config/types.rs:650`,
    `crates/capsem-core/src/net/policy_hook.rs:173`.
  - Detail: `hook.decision` and `policy.hook.*` parse, and
    `PolicyHookClient::decide` exists, but no non-test production caller was
    found.
  - Proof: if shipping, black-box hook-block-prevents-dispatch test with
    `policy_hook_events`; if not shipping, hidden/rejected hook UI/settings
    tests.
  - Sprint IDs: T8.1, T8.2, T8.3, T2.6.
  - T8 audit: Gibbs confirmed `SettingsFile` has no endpoint config,
    defaults expose no hook endpoint, service exposes only `/policy-hook/spec`,
    process reload carries merged policy but no hook client/endpoint, and no
    production caller invokes `PolicyHookClient::decide`.
  - Transfer status: deferred for `1.1.xxx`; T8.3 rejects new
    `policy.hook.*` writes/imports, keeps Spec0/client/audit infrastructure,
    and leaves endpoint propagation plus black-box hook E2E as post-1.1 work.

- [x] [P1] Hook response body cap is checked only after buffering the full
  body.
  - Paths: `crates/capsem-core/src/net/policy_hook.rs:253`,
    `crates/capsem-core/src/net/policy_hook.rs:292`,
    `crates/capsem-core/src/net/policy_hook.rs:237`.
  - Proof: raw TCP streaming server sends over-cap bytes without closing;
    assert immediate `ResponseTooLarge`.
  - Run: `cargo test -p capsem-core policy_hook -- --nocapture`.
  - Sprint IDs: T3.2.

- [x] [P1] Loopback detection is prefix-based and treats DNS lookalikes as
  local.
  - Paths: `crates/capsem-core/src/net/policy_hook.rs:326`,
    `crates/capsem-core/src/net/policy_hook.rs:310`.
  - Proof: reject `127.evil.example`, `127.0.0.1.evil`,
    `localhost.evil`, `https://127.0.0.1@evil.example`; allow exact loopbacks.
  - Run: `cargo test -p capsem-core policy_hook -- --nocapture`.
  - Sprint IDs: T3.1.

- [x] [P1] Fail-closed config can silently fail open.
  - Paths: `crates/capsem-core/src/net/policy_hook.rs:73`,
    `crates/capsem-core/src/net/policy_hook.rs:106`,
    `crates/capsem-core/src/net/policy_hook.rs:128`.
  - Detail: fallback `allow` becomes fail-open; `rewrite` produces no rewrite
    fields.
  - Proof: validation rejects fallback `allow`/`rewrite`, accepts only
    `block`/`ask`.
  - Run: `cargo test -p capsem-core policy_hook -- --nocapture`.
  - Sprint IDs: T3.3.

- [x] [P1] Spec0 semantic validation is incomplete.
  - Paths: `crates/capsem-core/src/net/policy_hook_spec.rs:123`,
    `crates/capsem-core/src/net/policy_hook_spec.rs:301`,
    `crates/capsem-core/src/net/policy_hook.rs:302`.
  - Detail: request `subject`, `preview`, and `hashes` are open
    `serde_json::Value`; response rewrite semantics are accepted too broadly.
  - Proof: request object-shape tests and response decision/rewrite semantic
    tests.
  - Run: `cargo test -p capsem-core policy_hook_spec policy_hook -- --nocapture`.
  - Sprint IDs: T3.3.

- [x] [P1] Hook fallback audit rows record fallback as a real decision.
  - Paths: `crates/capsem-core/src/net/policy_hook.rs:354`,
    `crates/capsem-core/src/net/policy_hook.rs:358`.
  - Proof: SQL assertions for timeout/schema/status/body-cap fallback with
    `decision IS NULL`, `status = error`, and `fallback = block|ask`.
  - Run: `cargo test -p capsem-core policy_hook -- --nocapture`.
  - Sprint IDs: T3.4, T6.3.

- [ ] [P1] Asset version selection uses lexicographic string comparison.
  - Paths: `crates/capsem-core/src/asset_manager.rs:454`,
    `crates/capsem-core/src/asset_manager.rs:759`.
  - Detail: same-day unpadded versions such as `2026.0415.10` can sort before
    `2026.0415.2`.
  - Proof: resolver/merge tests with `.2`, `.9`, `.10` same-day asset
    versions.
  - Run: `cargo test -p capsem-core asset_manager -- --nocapture`.
  - Sprint IDs: T1.1, T1.2.

- [ ] [P2] Asset cleanup still skips arch subdirectories and legacy dirs.
  - Paths: `crates/capsem-core/src/asset_manager.rs:563`,
    `crates/capsem-service/src/main.rs:4485`,
    `crates/capsem-core/src/asset_manager.rs:1279`.
  - Proof: arch-subdir stale hash fixture plus `v1.0.*` directory removal
    fixture.
  - Run: `cargo test -p capsem-core asset_manager -- --nocapture`.
  - Sprint IDs: T1.4, T10.2.

- [x] [P2] MCP Policy V2 telemetry still says audit-only and `deny`.
  - Paths: `crates/capsem-core/src/net/mitm_proxy/mcp_frame.rs:575`,
    `crates/capsem-core/src/net/mitm_proxy/mcp_frame.rs:740`,
    `crates/capsem-core/src/net/mitm_proxy/mcp_frame.rs:945`,
    `crates/capsem-core/src/net/mitm_proxy/mcp_frame.rs:603`,
    `crates/capsem-core/src/net/mitm_proxy/mcp_frame.rs:749`.
  - Proof: block/allow/audit/default telemetry tests with normalized
    action/mode.
  - Run: `cargo test -p capsem-core mcp_frame -- --nocapture`.
  - Sprint IDs: T3.6, T8.5.

- [x] [P3] Policy V2 benchmarks can silently measure no-match paths.
  - Paths: `crates/capsem-core/benches/policy_v2.rs:97`,
    `crates/capsem-core/benches/policy_v2.rs:107`,
    `crates/capsem-core/benches/policy_v2.rs:140`.
  - Proof: pre-bench `assert!(...unwrap().is_some())` for each subject.
  - Run: `cargo bench -p capsem-core --bench policy_v2 -- --sample-size 10 --warm-up-time 0.1 --measurement-time 0.2`.
  - Sprint IDs: T3.7, T10.4.

## T3 Execution Audit, 2026-05-10

Agent: Lagrange (`019e12fd-d72b-7ad1-ac5e-f1907235feac`)

Status: completed; no edits made by the agent. Findings captured during T3
implementation.

- [x] T3.1 confirmed prefix loopback detection and userinfo edge cases; fixed
  with exact localhost or parsed loopback IP validation plus URL userinfo
  rejection.
- [x] T3.2 confirmed `response.bytes().await` buffered before cap; fixed by
  passing `body_cap_bytes` into transport and reading the response stream with
  an accumulating cap.
- [x] T3.3 confirmed fail-closed fallback accepted `allow`/`rewrite`; fixed by
  serde validation and runtime sanitization to `block`/`ask`.
- [x] T3.3 confirmed Spec0 object-shape and rewrite semantic gaps; fixed with
  request/response semantic validators before callout/audit.
- [x] T3.4 confirmed fallback audit rows wrote fallback as a real decision;
  fixed so transport/schema/status/body-cap failures write `decision = NULL`,
  `status = error`, and `fallback = block|ask`.
- [x] T3.5/T3.6 MCP notification and telemetry items are implemented in
  `mcp_frame` with adversarial notification and normalized telemetry tests.
- [x] T3.7 benchmark no-match drift covered by pre-bench
  `find_matching_rule(...).is_some()` assertions.

## Tests Run

- `cargo test -p capsem-core policy_hook -- --nocapture` (23 passed; rerun
  escalated because the raw TCP streaming test binds a localhost listener).
- `cargo test -p capsem-core policy_hook_spec -- --nocapture` (6 passed).
- `cargo test -p capsem-core mcp_frame -- --nocapture` (50 passed).
- `cargo test -p capsem-core mcp_endpoint -- --nocapture` (9 passed).
- `cargo test -p capsem-logger mcp_call -- --nocapture` (15 passed).
- `cargo bench -p capsem-core --bench policy_v2 -- --sample-size 10 --warm-up-time 0.1 --measurement-time 0.2`
  (completed; all benchmark setup assertions passed).

## Tests Not Run

- Full VM/E2E proof remains owned by T8/T10.
