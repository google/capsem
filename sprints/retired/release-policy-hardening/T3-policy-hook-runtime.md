# T3: Policy Hook Runtime Hardening

## Objective

Harden the policy hook client and Policy V2 runtime so the fail-closed story is
true under malicious inputs, oversized responses, schema failures, MCP
notifications, and audit queries. Hook failures must be distinguishable from
valid hook denials.

## Owned Files

- `crates/capsem-core/src/net/policy_hook.rs`
- `crates/capsem-core/src/net/policy_hook/tests.rs`
- `crates/capsem-core/src/net/policy_hook_spec.rs`
- `crates/capsem-core/src/net/policy_hook_spec/tests.rs`
- `crates/capsem-core/src/net/mitm_proxy/mcp_frame.rs`
- `crates/capsem-core/src/net/mitm_proxy/mcp_frame/tests.rs`
- `crates/capsem-core/src/net/policy_config/types.rs`
- `crates/capsem-core/src/net/mitm_proxy/policy_v2_http_hook.rs`
- `crates/capsem-core/src/net/mitm_proxy/policy_v2_model.rs`
- `crates/capsem-core/src/net/dns/server.rs`
- `crates/capsem-core/src/mcp/policy.rs`
- `crates/capsem-core/benches/policy_v2.rs`
- `crates/capsem-logger/src/events.rs`
- `config/policy-hook-openapi.json`

## Findings

- [P1] `policy_hook.rs:326` treats any hostname starting with `127.` as
  loopback. `http://127.evil.example/...` can bypass HTTPS and bearer-token
  requirements while resolving through DNS.
- [P1] `policy_hook.rs:292` buffers the whole response with
  `response.bytes().await` before checking the body cap.
- [P1] MCP notification frames can bypass request policy and telemetry:
  `mcp_frame.rs:156` dispatches notification frames before request policy is
  enforced, so a `tools/call` sent without JSON-RPC id can reach the aggregator
  with no `mcp_calls` row.
- [P2] `policy_hook.rs:73` accepts any `HookDecision` for
  `fail_closed_decision`; `allow` silently becomes fail-open, and `rewrite`
  can be returned without rewrite fields.
- [P2] Spec0 has strict envelope serde, but `subject`, `preview`, and `hashes`
  are open `serde_json::Value` objects and response decision semantics are not
  validated.
- [P2] `policy_hook.rs:354` audits transport/schema fail-closed fallback rows
  with `decision = Some(response.decision)`, conflicting with the logger
  contract that says transport/schema failures use `decision = None`.
- [P2] `PolicyHookClient` and `PolicyHookEndpoint` appear test-only in
  practice. No production MCP/HTTP/DNS/model call site invokes
  `PolicyHookClient::decide`.
- [P2] `policy.hook` and `hook.decision` rules parse and benchmark, but no
  production runtime path evaluates hook-decision policy.
- [P2] MCP Policy V2 telemetry reports the provider as `audit_only` even for
  enforced decisions, maps block to action `deny` rather than `block`, and
  drops non-blocking Policy V2 matches before logging.
- [P3] `policy_v2.rs` benchmark setup unwraps only `Result`, not `Option`, so
  benchmark setup can drift into no-match measurements.

## Swarm Transfer Tracker

| Source | Priority | Owner task | Required transfer point | Required proof |
|---|---:|---|---|---|
| FD04 core-policy-assets | P0 | T3.5 | MCP notifications can bypass request policy and telemetry because notifications dispatch before block/log handling. | Adversarial no-id `tools/call` notification test proves no aggregator dispatch, no argument leak, and intended denial/audit behavior. |
| FD04 core-policy-assets | P0 | T8.1, T8.2, T8.3 | Hook policy parses in core config but no production caller was found. | T8 scope decision either wires black-box dispatch proof or T2/T4 hide/document hook as infrastructure-only. |
| FD04 core-policy-assets | P1 | T3.2 | Hook response body cap is checked only after buffering the full body. | Raw TCP streaming over-cap test returns `ResponseTooLarge` without unbounded buffering or hang. |
| FD04 core-policy-assets | P1 | T3.1 | Loopback detection is prefix-based and accepts DNS lookalikes. | Tests reject `127.evil.example`, `127.0.0.1.evil`, `localhost.evil`, and userinfo tricks; exact loopbacks pass. |
| FD04 core-policy-assets | P1 | T3.3 | Fail-closed config can silently fail open by accepting fallback `allow` or malformed `rewrite`. | Validation rejects fallback `allow`/`rewrite` and accepts only release-approved fallback decisions. |
| FD04 core-policy-assets | P1 | T3.3 | Spec0 semantic validation is incomplete for request object shape and response rewrite semantics. | Request object-shape and response decision/rewrite tests pass. |
| FD04 core-policy-assets | P1 | T3.4, T6.3 | Hook fallback audit rows record fallback as a real decision. | SQL asserts timeout/schema/status/body-cap fallback rows have `decision IS NULL`, `status = error`, and fallback reason. |
| FD04 core-policy-assets | P2 | T3.6 | MCP Policy V2 telemetry still says audit-only and `deny`. | Block/allow/audit/default telemetry tests use normalized action/mode or document a single boundary. |
| FD04 core-policy-assets | P3 | T3.7 | Policy V2 benchmarks can silently measure no-match paths. | Pre-bench assertions require matched rules before measurement. |
| FD05 service-process | P1 | T8.1, T8.2, T8.3 | Hook dispatch is not integrated through service/process. | Same T8 shipping-scope proof or deferred UI/docs proof. |
| FD07 mcp-policy-boundary | P0 | T3.5 | No-id `tools/call` notification bypass can execute tools without policy blocking or telemetry. | Same adversarial notification test plus sensitive-argument log absence proof. |
| FD07 mcp-policy-boundary | P2 | T3.6 | MCP telemetry action/mode naming is inconsistent for enforced blocks. | Normalized action/mode tests pass or docs/tooling translate explicitly. |
| FD08 telemetry-session | P1 | T3.4 | Hook failure audit rows look like real hook decisions. | Extend malformed schema/timeout/status/body-cap tests to assert fallback contract. |

## Task List

### T3.1 Local Hook Endpoint Security

- [x] Replace hostname-prefix localhost detection with exact `localhost` or
  parsed `IpAddr::is_loopback`.
- [x] Reject `127.evil.example`, `127.0.0.1.evil`, `localhost.evil`, and
  `https://127.0.0.1@evil.example`.
- [x] Allow `127.0.0.1`, `[::1]`, and exact `localhost`.
- [x] Confirm HTTPS and bearer-token requirements still apply to all non-local
  endpoints.

### T3.2 Bounded Hook Response Reading

- [x] Pass the response cap into the reqwest transport path.
- [x] Read hook responses with a capped stream loop.
- [x] Return a `ResponseTooLarge`-style error immediately once
  `body_cap_bytes` is exceeded.
- [x] Add a raw TCP HTTP test server that streams over-cap bytes without
  closing the response.
- [x] Assert the client does not hang and does not buffer unbounded data.

### T3.3 Fail-Closed and Spec0 Semantics

- [x] Restrict fallback decisions to `block` and/or `ask` via a narrow enum or
  custom serde validation.
- [x] Reject `allow` and `rewrite` fallback config unless an explicit fail-open
  setting is introduced.
- [x] Validate hook response semantics: `rewrite` requires both
  `rewrite_target` and `rewrite_value`.
- [x] Reject non-rewrite decisions carrying rewrite fields.
- [x] Validate `subject`, `preview`, and `hashes` are JSON objects, not arrays,
  scalars, or null.
- [x] Confirmed no `config/policy-hook-openapi.json` update is needed; the
  runtime semantic checks enforce the existing object/rewrite contract without
  changing the published schema artifact.

### T3.4 Audit Correctness

- [x] On transport/schema/status/timeout/body-cap failures, write
  `decision = None`, `status = "error"`, and `fallback = Some("block"|"ask")`.
- [x] Add SQL/log assertions for valid hook block, timeout fallback, schema
  fallback, status fallback, and body-cap fallback rows.
- [x] Preserve machine-readable status/error/fallback fields so UI/session
  tooling can explain fallback reasons; T6/T10 still own human-facing timeline
  proof.

### T3.5 MCP Notification Policy Bypass

- [x] Reject or drop notification frames whose method is not an allowed
  `notifications/*` method.
- [x] Prefer an allowlist containing only `notifications/initialized` unless
  another notification is required by protocol tests.
- [x] Add an adversarial `tools/call` notification test that proves no
  aggregator dispatch and no argument leak occur.
- [x] Add telemetry assertion that bypass attempts are denied/audited according
  to the intended MCP policy contract.

### T3.6 MCP Policy V2 Telemetry Semantics

- [x] Add an enforce mode for MCP Policy V2 decisions rather than always
  logging provider `audit_only`.
- [x] Normalize MCP Policy V2 action naming with HTTP/DNS/model (`block`
  rather than `deny`) or document a single translation boundary.
- [x] Preserve matched allow decisions in telemetry unless superseded by a
  higher-priority denial.
- [x] Add tests for block, allow, audit-only, and legacy/default fallback
  telemetry.

### T3.7 Bench Guardrails

- [x] Assert `find_matching_rule(...).unwrap().is_some()` outside every measured
  benchmark loop.
- [x] Add assertions so future benchmark subjects cannot silently benchmark
  no-match paths.

## Proof Matrix

| Category | Required proof |
|---|---|
| Unit/contract | hook URL validation, fallback decision parsing, Spec0 response validation. |
| Functional | hook client handles valid and invalid HTTP responses with bounded reads. |
| Adversarial | DNS lookalikes, oversized streaming body, timeout, bad rewrite, MCP notification `tools/call`. |
| Telemetry | hook fallback rows use `decision = None`; MCP Policy V2 action/mode is truthful. |
| Performance | policy benchmark asserts matching rule setup before measuring. |
| Missing/deferred | production hook dispatch is owned by T8; T3 hardens the library and local runtime boundaries. |

## Verification

- [x] `cargo test -p capsem-core policy_hook -- --nocapture` (23 passed; rerun
  escalated because the raw TCP streaming test binds a localhost listener).
- [x] `cargo test -p capsem-core policy_hook_spec -- --nocapture` (6 passed).
- [x] `cargo test -p capsem-core mcp_frame -- --nocapture` (50 passed).
- [x] `cargo test -p capsem-core mcp_endpoint -- --nocapture` (9 passed).
- [x] `cargo test -p capsem-logger mcp_call -- --nocapture` (15 passed).
- [x] `cargo bench -p capsem-core --bench policy_v2 -- --sample-size 10 --warm-up-time 0.1 --measurement-time 0.2`
  (completed; all benchmark setup assertions passed).
- [x] T8/T10 E2E: if hook dispatch ships, add one black-box MCP or MITM
  functional test proving a hook block prevents dispatch and writes
  `policy_hook_events`. T8.1 defers configured external hook dispatch for
  `1.1.1778445002`, so this command is not promoted to the final release gate.
- [ ] T8/T10 E2E: run non-hook Policy V2/MCP integration proof:
  `pytest tests/capsem-e2e/test_framed_mcp_mitm.py -k "policy_v2 or notification" -v`
  or its final VM-safe equivalent.

Note: one parallel cargo test attempt hit a codesign artifact lock while
`mcp_frame` and `policy_hook_spec` built together; both suites passed when
rerun sequentially.

## Exit Criteria

- [x] Non-local DNS names cannot bypass HTTPS/auth by starting with `127.`.
- [x] Hook response body cap is enforced while reading.
- [x] Fail-closed config cannot silently become fail-open.
- [x] SQL can distinguish valid hook block from fallback block.
- [x] MCP notification frames cannot bypass policy/telemetry in the framed
  runtime unit path.
- [x] T8/T10 still own VM/E2E proof for configured hook shipping scope and MCP
  policy integration; configured external hook dispatch is deferred for
  `1.1.1778445002`, and non-hook MCP policy integration remains the focused VM proof.
