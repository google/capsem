# Core Policy and Assets Findings

Status: completed, pending transfer into T1/T3/T8/T10.

Agent: Kant (`019e1264-dba6-7ae3-b34e-20edf051132d`)

## Scope

- `capsem-core` asset manifest resolution and compatibility.
- Policy V2 hook runtime and Spec0 validation.
- Body caps, fail-closed semantics, MCP frame assumptions.
- Benchmark coverage.

## Findings

- [ ] [P0] MCP notifications can bypass request policy and telemetry.
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

- [ ] [P0] Hook policy is accepted by core config but not wired to production
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

- [ ] [P1] Hook response body cap is checked only after buffering the full
  body.
  - Paths: `crates/capsem-core/src/net/policy_hook.rs:253`,
    `crates/capsem-core/src/net/policy_hook.rs:292`,
    `crates/capsem-core/src/net/policy_hook.rs:237`.
  - Proof: raw TCP streaming server sends over-cap bytes without closing;
    assert immediate `ResponseTooLarge`.
  - Run: `cargo test -p capsem-core policy_hook -- --nocapture`.
  - Sprint IDs: T3.2.

- [ ] [P1] Loopback detection is prefix-based and treats DNS lookalikes as
  local.
  - Paths: `crates/capsem-core/src/net/policy_hook.rs:326`,
    `crates/capsem-core/src/net/policy_hook.rs:310`.
  - Proof: reject `127.evil.example`, `127.0.0.1.evil`,
    `localhost.evil`, `https://127.0.0.1@evil.example`; allow exact loopbacks.
  - Run: `cargo test -p capsem-core policy_hook -- --nocapture`.
  - Sprint IDs: T3.1.

- [ ] [P1] Fail-closed config can silently fail open.
  - Paths: `crates/capsem-core/src/net/policy_hook.rs:73`,
    `crates/capsem-core/src/net/policy_hook.rs:106`,
    `crates/capsem-core/src/net/policy_hook.rs:128`.
  - Detail: fallback `allow` becomes fail-open; `rewrite` produces no rewrite
    fields.
  - Proof: validation rejects fallback `allow`/`rewrite`, accepts only
    `block`/`ask`.
  - Run: `cargo test -p capsem-core policy_hook -- --nocapture`.
  - Sprint IDs: T3.3.

- [ ] [P1] Spec0 semantic validation is incomplete.
  - Paths: `crates/capsem-core/src/net/policy_hook_spec.rs:123`,
    `crates/capsem-core/src/net/policy_hook_spec.rs:301`,
    `crates/capsem-core/src/net/policy_hook.rs:302`.
  - Detail: request `subject`, `preview`, and `hashes` are open
    `serde_json::Value`; response rewrite semantics are accepted too broadly.
  - Proof: request object-shape tests and response decision/rewrite semantic
    tests.
  - Run: `cargo test -p capsem-core policy_hook_spec policy_hook -- --nocapture`.
  - Sprint IDs: T3.3.

- [ ] [P1] Hook fallback audit rows record fallback as a real decision.
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

- [ ] [P2] MCP Policy V2 telemetry still says audit-only and `deny`.
  - Paths: `crates/capsem-core/src/net/mitm_proxy/mcp_frame.rs:575`,
    `crates/capsem-core/src/net/mitm_proxy/mcp_frame.rs:740`,
    `crates/capsem-core/src/net/mitm_proxy/mcp_frame.rs:945`,
    `crates/capsem-core/src/net/mitm_proxy/mcp_frame.rs:603`,
    `crates/capsem-core/src/net/mitm_proxy/mcp_frame.rs:749`.
  - Proof: block/allow/audit/default telemetry tests with normalized
    action/mode.
  - Run: `cargo test -p capsem-core mcp_frame -- --nocapture`.
  - Sprint IDs: T3.6, T8.5.

- [ ] [P3] Policy V2 benchmarks can silently measure no-match paths.
  - Paths: `crates/capsem-core/benches/policy_v2.rs:97`,
    `crates/capsem-core/benches/policy_v2.rs:107`,
    `crates/capsem-core/benches/policy_v2.rs:140`.
  - Proof: pre-bench `assert!(...unwrap().is_some())` for each subject.
  - Run: `cargo bench -p capsem-core --bench policy_v2 -- --sample-size 10 --warm-up-time 0.1 --measurement-time 0.2`.
  - Sprint IDs: T3.7, T10.4.

## Tests Not Run

- Static code-reading investigation only; no tests were run.
