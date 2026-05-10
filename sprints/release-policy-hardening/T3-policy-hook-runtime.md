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

## Task List

### T3.1 Local Hook Endpoint Security

- [ ] Replace hostname-prefix localhost detection with exact `localhost` or
  parsed `IpAddr::is_loopback`.
- [ ] Reject `127.evil.example`, `127.0.0.1.evil`, `localhost.evil`, and
  `https://127.0.0.1@evil.example`.
- [ ] Allow `127.0.0.1`, `[::1]`, and exact `localhost`.
- [ ] Confirm HTTPS and bearer-token requirements still apply to all non-local
  endpoints.

### T3.2 Bounded Hook Response Reading

- [ ] Pass the response cap into the reqwest transport path.
- [ ] Read hook responses with a capped `chunk()` loop.
- [ ] Return a `ResponseTooLarge`-style error immediately once
  `body_cap_bytes` is exceeded.
- [ ] Add a raw TCP HTTP test server that streams over-cap bytes without
  closing the response.
- [ ] Assert the client does not hang and does not buffer unbounded data.

### T3.3 Fail-Closed and Spec0 Semantics

- [ ] Restrict fallback decisions to `block` and/or `ask` via a narrow enum or
  custom serde validation.
- [ ] Reject `allow` and `rewrite` fallback config unless an explicit fail-open
  setting is introduced.
- [ ] Validate hook response semantics: `rewrite` requires both
  `rewrite_target` and `rewrite_value`.
- [ ] Reject non-rewrite decisions carrying rewrite fields.
- [ ] Validate `subject`, `preview`, and `hashes` are JSON objects, not arrays,
  scalars, or null.
- [ ] Update `config/policy-hook-openapi.json` if the response schema changes.

### T3.4 Audit Correctness

- [ ] On transport/schema/status/timeout/body-cap failures, write
  `decision = None`, `status = "error"`, and `fallback = Some("block"|"ask")`.
- [ ] Add SQL/log assertions for valid hook block, timeout fallback, schema
  fallback, status fallback, and body-cap fallback rows.
- [ ] Ensure UI/session tooling can explain fallback reason from status/error
  fields without confusing it with a valid hook block.

### T3.5 MCP Notification Policy Bypass

- [ ] Reject or drop notification frames whose method is not an allowed
  `notifications/*` method.
- [ ] Prefer an allowlist containing only `notifications/initialized` unless
  another notification is required by protocol tests.
- [ ] Add an adversarial `tools/call` notification test that proves no
  aggregator dispatch and no argument leak occur.
- [ ] Add telemetry assertion that bypass attempts are denied/audited according
  to the intended MCP policy contract.

### T3.6 MCP Policy V2 Telemetry Semantics

- [ ] Add an enforce mode for MCP Policy V2 decisions rather than always
  logging provider `audit_only`.
- [ ] Normalize MCP Policy V2 action naming with HTTP/DNS/model (`block`
  rather than `deny`) or document a single translation boundary.
- [ ] Preserve matched allow decisions in telemetry unless superseded by a
  higher-priority denial.
- [ ] Add tests for block, allow, audit-only, and legacy/default fallback
  telemetry.

### T3.7 Bench Guardrails

- [ ] Assert `find_matching_rule(...).unwrap().is_some()` outside every measured
  benchmark loop.
- [ ] Add comments/tests so future benchmark subjects cannot silently benchmark
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

- [ ] `cargo test -p capsem-core policy_hook -- --nocapture`
- [ ] `cargo test -p capsem-core policy_hook_spec -- --nocapture`
- [ ] `cargo test -p capsem-core mcp_frame -- --nocapture`
- [ ] `cargo bench -p capsem-core --bench policy_v2 -- --sample-size 10 --warm-up-time 0.1 --measurement-time 0.2`
- [ ] If T8 wires hook dispatch: add one black-box MCP or MITM functional test
  proving a hook block prevents dispatch and writes `policy_hook_events`.

## Exit Criteria

- [ ] Non-local DNS names cannot bypass HTTPS/auth by starting with `127.`.
- [ ] Hook response body cap is enforced while reading.
- [ ] Fail-closed config cannot silently become fail-open.
- [ ] SQL can distinguish valid hook block from fallback block.
- [ ] MCP notification frames cannot bypass policy/telemetry.
