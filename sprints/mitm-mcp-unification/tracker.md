# Sprint: MITM MCP Unification

## Status

T0 baseline and wire-format gate complete. Framed MCP over `vsock:5002`
is selected for T1+ because it is operationally simple, preserves the
existing aggregator/policy execution path in the spike, carries
per-frame process attribution, and does not materially regress the fresh
same-hardware `vsock:5003` baseline.

## Tasks

- [x] T0: Baseline and wire-format gate
  - [x] Re-baseline the current `vsock:5003` MCP path on this hardware
  - [x] Save or record the pre-change baseline artifact
  - [x] Prototype framed JSON-RPC over `vsock:5002`
  - [x] Run `mcp-load` against current and framed paths
  - [x] Record rps/p99 results below
  - [x] Confirm framed or pause the sprint with a documented blocker/regression
- [x] T1: Framed MCP parser/interpreter
  - [x] Add `Protocol::McpFrame` sniffing on `vsock:5002`
  - [x] Add bounded MCP frame codec with `u32 total_frame_len_be`
  - [x] Specify and test `u32` per-connection monotonic `stream_id` lifecycle
  - [x] Reserve `stream_id=0` for notifications and skip pending-map insertion
  - [x] Carry per-frame `process_name`
  - [x] Add bounded JSON-RPC parser
  - [x] Add MCP method interpreter
  - [x] Add method metrics
  - [x] Add parser/interpreter tests and fixtures, including corrupted frame behavior
  - [x] Run scoped `mcp-load` regression check
- [x] T2: Decision provider abstraction
  - [x] Add local decision provider for MCP endpoint
  - [x] Keep provider shape compatible with future remote corp decision forwarding
  - [x] Keep v1 decisions to Allow/Deny
  - [x] Add `mcp_calls.policy_mode`, `policy_action`, `policy_rule`, and `policy_reason`
  - [x] Update session DB readers and inspect output for the new policy fields
  - [x] Encode the audit-only truth table in tests
  - [x] Add per-method decision granularity tests
  - [x] Preserve request preview/hash fields for future corp provider
- [x] T3: MITM MCP endpoint
  - [x] Add `McpEndpointState`
  - [x] Wire endpoint into `MitmProxyConfig`
  - [x] Enforce non-`tools/call` default timeout `60s`
  - [x] Enforce `tools/call` default/ceiling `300s` with per-tool catalog overrides
  - [x] Dispatch through `AggregatorClient`
  - [x] Emit framed `mcp_calls` from MITM only
  - [x] Ensure aggregator has no DB access
  - [x] Run scoped `mcp-load` regression check
- [x] T4: Atomic transport cutover and 5003 removal
  - [x] Rewrite guest `capsem-mcp-server` to framed transport over `vsock:5002`
  - [x] Preserve per-frame process attribution
  - [x] Add reconnect and in-flight disconnect behavior
  - [x] Add non-idempotent `tools/call` disconnect warning in code comments
  - [x] Add disconnect counter
  - [x] Delete `crates/capsem-core/src/mcp/gateway.rs`
  - [x] Delete `VSOCK_PORT_MCP_GATEWAY`
  - [x] Delete 5003 dispatch and tests
  - [x] Remove stale "MCP gateway" terminology for guest MCP path
  - [x] Run scoped `mcp-load` regression check
- [ ] T5: Phase 1 outbound verification
  - [ ] Test builtin HTTP domain policy still applies through the framed MITM MCP path
  - [ ] Test builtin HTTP direct `net_events` still emit
  - [ ] Test configured external MCP tool calls are inspected at the MITM MCP boundary, including request policy, response policy, previews, and `mcp_calls` telemetry
  - [ ] Separately name any downstream network side effects performed inside host-side external MCP server processes, without conflating them with guest-agent MCP inspection
- [ ] T6: End-to-end verification
  - [x] `tools/list`
  - [x] builtin `tools/call`
  - [x] external MCP tool when configured
  - [x] malformed JSON
  - [x] oversized request
  - [x] corrupted frame reconnect/error behavior
  - [x] non-`tools/call` and `tools/call` timeout JSON-RPC errors
  - [x] notification interleaving
  - [x] concurrent calls from N distinct process names preserve `mcp_calls.process_name`
  - [x] policy denial
  - [x] post-resume reconnect
  - [x] `capsem-doctor` MCP tests
  - [x] no `vsock:5003`
- [ ] T7: Baseline artifact update
  - [ ] Save post-cutover baseline as `benchmarks/mcp-load/baseline-after-mitm-unification.json`
  - [ ] Decide whether `benchmarks/mcp-load/baseline.json` remains historical or becomes the new canonical file
- [ ] Testing gate
  - [x] `cargo test -p capsem-core mitm_proxy`
  - [x] `cargo test -p capsem-core mcp`
  - [x] `cargo test -p capsem-agent mcp_server`
  - [x] `cargo test -p capsem-proto`
  - [ ] `just test-mcp`
  - [ ] `just smoke`
  - [ ] `just test`
  - [x] `just inspect-session`
- [x] Changelog
- [ ] Commit series

## T0 Benchmark Results

Fill this in before production implementation.

| Transport | Concurrency | RPS | p50 | p95 | p99 | Notes |
|---|---:|---:|---:|---:|---:|---|
| current 5003, fresh same-hardware baseline | 1 | 2069.2 | 0.45 | 0.65 | 1.02 | 0 errors |
| current 5003, fresh same-hardware baseline | 10 | 8748.2 | 1.08 | 1.66 | 2.29 | 0 errors |
| current 5003, fresh same-hardware baseline | 50 | 9817.5 | 4.89 | 6.46 | 8.33 | 0 errors |
| current 5003, fresh same-hardware baseline | 200 | 7909.6 | 22.20 | 43.38 | 58.28 | 0 errors |
| framed over 5002 | 1 | 2247.5 | 0.43 | 0.56 | 0.69 | 0 errors |
| framed over 5002 | 10 | 9166.5 | 1.06 | 1.48 | 1.74 | 0 errors |
| framed over 5002 | 50 | 9187.1 | 5.23 | 7.28 | 8.98 | 0 errors |
| framed over 5002 | 200 | 8339.3 | 22.55 | 32.81 | 40.19 | 0 errors |

Chosen transport: framed JSON-RPC over `vsock:5002`.

Reason: compared with the fresh `vsock:5003` baseline, framed delivered
+8.6% / +4.8% / -6.4% / +5.4% rps at concurrency 1/10/50/200 and
-31.9% / -23.9% / +7.8% / -31.0% p99. The only regression is the
c=50 point, and it stays within the T0 tolerance recorded here: reject
the candidate if any level shows >10% rps regression or >2x p99
regression versus the fresh baseline. Framed also carried per-frame
`process_name`, used a bounded length-prefixed envelope, and reused the
existing aggregator/policy path in the spike.

Pre-change baseline artifact:
`benchmarks/mcp-load/baseline-pre-mitm-unification.json`.

Framed T0 artifact:
`benchmarks/mcp-load/baseline-framed-mitm-unification-t0.json`.

Post-change baseline artifact: pending T4 cutover; target remains
`benchmarks/mcp-load/baseline-after-mitm-unification.json`.

## Hot-Path Bench Log

Record every scoped `mcp-load` run after a hot-path commit.

| Commit/task | Command | RPS | p99 | Result | Notes |
|---|---|---:|---:|---|---|
| T1 parser/interpreter | `target/debug/capsem --uds-path /Users/elie/.capsem/run/service.sock exec mitm-t1-full-bench 'CAPSEM_MCP_TRANSPORT=framed CAPSEM_BENCH_MCP_DURATION=10 capsem-bench mcp-load'` | 2028.5 / 9768.3 / 10028.6 / 9457.0 | 0.8 / 1.6 / 6.9 / 25.9 | pass | 10s per level, concurrency 1/10/50/200, 0 errors; ran through normal `capsem exec` with no explicit timeout |
| T2 decision provider | `target/debug/capsem --uds-path /Users/elie/.capsem/run/service.sock exec mitm-t2-bench 'CAPSEM_MCP_TRANSPORT=framed CAPSEM_BENCH_MCP_DURATION=10 capsem-bench mcp-load'` | 2125.1 / 9610.9 / 9892.5 / 9551.3 | 0.7 / 1.6 / 7.6 / 27.9 | pass | 10s per level, concurrency 1/10/50/200, 0 errors; DB sanity query showed populated `audit_only` policy fields for 311799 `local__echo` calls |
| T2 hardening | `capsem_exec mitm-t2-hardening-enforce-bench 'CAPSEM_MCP_TRANSPORT=framed CAPSEM_BENCH_MCP_DURATION=10 capsem-bench mcp-load'` | 2067.1 / 9865.1 / 10304.1 / 9927.4 | 0.6 / 1.5 / 6.3 / 23.6 | pass | 10s per level, concurrency 1/10/50/200, 0 errors; DB sanity query showed populated `audit_only` policy fields for 321638 `local__echo` calls |
| T3 endpoint | `capsem_exec mitm-t3-endpoint-bench 'CAPSEM_MCP_TRANSPORT=framed CAPSEM_BENCH_MCP_DURATION=10 capsem-bench mcp-load'` | 2186.3 / 9984.0 / 10217.6 / 9891.9 | 0.6 / 1.5 / 6.0 / 25.6 | pass | 10s per level, concurrency 1/10/50/200, 0 errors; DB sanity query showed populated `audit_only` policy fields for 322799 `local__echo` calls |
| T4 atomic cutover | `just exec "CAPSEM_BENCH_MCP_DURATION=10 capsem-bench mcp-load && cat /tmp/capsem-benchmark.json"` | 2078.1 / 9565.0 / 9644.3 / 9213.1 | 0.7 / 1.6 / 6.7 / 28.6 | pass | 10s per level, concurrency 1/10/50/200, 0 errors, no `CAPSEM_MCP_TRANSPORT` override; `capsem-mcp-server` logged framed relay on port 5002; `session.db` sanity query showed 305008 `mcp_calls` with populated `audit_only`/`allow` policy fields and `process_name=python3` |
| T4 coverage hardening | `just exec "CAPSEM_BENCH_MCP_DURATION=10 capsem-bench mcp-load && cat /tmp/capsem-benchmark.json"` | 2133.9 / 9490.1 / 9716.3 / 9074.1 | 0.7 / 1.6 / 6.5 / 33.5 | pass | 10s per level, concurrency 1/10/50/200, 0 errors, no transport override; compared with the T4 cutover run this is +2.7% / -0.8% / +0.7% / -1.5% rps and -3.7% / +0.4% / -2.8% / +17.2% p99; `session.db` sanity query on `witty-griffin-tmp` showed 304147 `mcp_calls` with populated `request_id`, `process_name`, `audit_only`, and `allow` fields |

## Notes

- `capsem-gateway` is unrelated to this sprint.
- Chosen target is framed JSON-RPC over `vsock:5002`; HTTP is not part of the execution plan.
- Aggregator must remain low-privilege and DB-free.
- MITM writes guest MCP `mcp_calls`; aggregator emits structured stderr diagnostics only.
- `mcp_calls.process_name` must come from per-frame attribution.
- Framed `net_events` use a synthetic internal endpoint identity; no DNS/upstream forwarding.
- Audit-only policy fields are in scope for T2 so the truth table is testable.
- Phase 1 inspects all guest-agent MCP traffic, including configured
  external MCP tools, at the framed MITM MCP boundary. T5 must prove that
  boundary with policy and telemetry tests, plus separately document any
  downstream host-side network side effects performed inside external MCP
  server processes.
- T4 is an atomic release unit: guest transport cutover and 5003 deletion must land together.
- T0 measurement caveat, resolved during T1: `target/debug/capsem exec
  ... capsem-bench mcp-load` previously tripped a fixed process-layer
  exec watchdog for long-running benchmark commands. T1 removes the
  hidden exec watchdog and makes `capsem exec` / `capsem run` wait for
  completion unless the user passes an explicit `--timeout`.

## T0 Update - 2026-05-07

T0 is closed. The fresh current-path baseline and framed-path candidate
were both captured on this machine with 4 GB RAM / 2 CPU benchmark VMs.
The framed candidate uses `CAPSEM_MCP_TRANSPORT=framed`, connects the
guest MCP relay to `vsock:5002`, sends bounded `MC` frames with per-frame
`process_name`, and routes through the existing host MCP
aggregator/policy/telemetry path. That keeps the T0 comparison focused
on the guest-host wire transport rather than mixing in a new dispatcher.

Decision: proceed with framed JSON-RPC over `vsock:5002` for T1+. It met
the T0 tolerance and improved the p99 tail at the highest concurrency.
The next sprint should keep the shared frame envelope but replace the
borrowed gateway handler with the real parser/interpreter shape:
bounded JSON-RPC parser, MCP method interpreter, stream-id lifecycle
tests, notification handling, duplicate-id/error behavior, and scoped
`mcp-load` after each hot-path commit.

Verification run for the T0 checkpoint:
- `cargo test -p capsem-proto mcp_frame -- --nocapture`
- `cargo test -p capsem-core mitm_proxy::protocol -- --nocapture`
- `cargo check -p capsem-agent --bin capsem-mcp-server`
- `cargo check -p capsem-process`
- `cargo check -p capsem-service`
- `PYTHONPYCACHEPREFIX=/private/tmp/capsem-pycache python3 -m py_compile guest/artifacts/capsem_bench/mcp_load.py`
- `cargo test -p capsem-service preserve -- --nocapture`
- Session DB sanity query on `upbeat-cedar-tmp`: framed run emitted
  `mcp_calls` rows with `process_name=python3`, `method=tools/call`,
  `tool_name=local__echo`, and `decision=allowed`.

## T1 Update - 2026-05-07

T1 is implemented. The framed MCP parser now rejects invalid transport
state before dispatch, including reserved flags, invalid notification
stream/id combinations, duplicate in-flight request stream ids, and
non-monotonic request stream ids after completion. Bounded JSON-RPC
parsing validates `jsonrpc: "2.0"` and method shape before the MCP
method interpreter extracts known method details for `tools/call`,
`resources/read`, and `prompts/get`; unknown and notification methods
still produce coarse method metrics without blocking the stream.

T1 also closes the benchmark feedback-loop blocker found in T0: exec
duration is user work, not transport liveness. The service accepts
optional `timeout_secs`; the CLI omits it by default; `send_ipc_command`
waits indefinitely when no timeout is set; and the process exec handler
waits for `ExecDone` instead of synthesizing a fixed watchdog failure.
Quick file read/write operations keep the short retry watchdog because
they are bounded request/response operations rather than arbitrary user
commands.

Verification run for the T1 checkpoint:
- `cargo test -p capsem-core mitm_proxy::mcp_frame -- --nocapture`
- `cargo test -p capsem-proto mcp_frame -- --nocapture`
- `cargo test -p capsem-process ipc::tests -- --nocapture`
- `cargo test -p capsem-service api::tests::exec_request -- --nocapture`
- `cargo test -p capsem-service api::tests::run_request -- --nocapture`
- `cargo test -p capsem parse_exec -- --nocapture`
- `cargo test -p capsem parse_run -- --nocapture`
- `cargo test -p capsem request_serde -- --nocapture`
- `cargo test -p capsem run_request_env_omitted_when_none -- --nocapture`
- `cargo check -p capsem-agent --bins`
- `cargo check --workspace --all-targets`
- `git diff --check`
- scoped `mcp-load` regression: `target/debug/capsem --uds-path
  /Users/elie/.capsem/run/service.sock exec mitm-t1-full-bench
  'CAPSEM_MCP_TRANSPORT=framed CAPSEM_BENCH_MCP_DURATION=10
  capsem-bench mcp-load'` passed with 0 errors at concurrency
  1/10/50/200; rps 2028.5 / 9768.3 / 10028.6 / 9457.0 and p99
  0.8 / 1.6 / 6.9 / 25.9 ms.

## T2 Update - 2026-05-07

T2 is implemented. The framed MCP path now builds an owned decision
request from the interpreter summary before dispatch. The request shape
is serializable for the future remote corp decision provider and carries
process name, method classification, server/tool/resource/prompt
identity, request preview, and BLAKE3 request hash. The local v1 provider
runs in `audit_only` mode and emits only `allow` or `deny` actions:
policy warn remains `allow`, blocked tools become per-tool denies, and
resource/prompt reads use server-level policy.

`mcp_calls` now has nullable `policy_mode`, `policy_action`,
`policy_rule`, and `policy_reason` columns. The logger writer stores
them, the reader returns them, raw inspect/schema output can select them,
and session triage includes them for MCP denied/error rows. Existing
gateway logging still works with empty policy fields; the framed path
passes populated fields.

Verification run for the T2 checkpoint:
- `cargo test -p capsem-core mitm_proxy::mcp_frame -- --nocapture`
- `cargo test -p capsem-core mcp::gateway -- --nocapture`
- `cargo test -p capsem-logger --lib migrate_mcp_calls_policy_fields_idempotent -- --nocapture`
- `cargo test -p capsem-logger --lib mcp_call_insert_populates_row -- --nocapture`
- `cargo test -p capsem-logger --test roundtrip mcp_call_roundtrip -- --nocapture`
- `cargo check --workspace --all-targets`
- Session DB sanity query on `mitm-t2-bench`: grouped `mcp_calls`
  showed `policy_mode=audit_only`, `policy_action=allow`,
  `policy_rule=mcp.tool.local__echo`, and populated reasons for the
  framed benchmark calls.
- scoped `mcp-load` regression: `target/debug/capsem --uds-path
  /Users/elie/.capsem/run/service.sock exec mitm-t2-bench
  'CAPSEM_MCP_TRANSPORT=framed CAPSEM_BENCH_MCP_DURATION=10
  capsem-bench mcp-load'` passed with 0 errors at concurrency
  1/10/50/200; rps 2125.1 / 9610.9 / 9892.5 / 9551.3 and p99
  0.7 / 1.6 / 7.6 / 27.9 ms.

## T2 Hardening Update - 2026-05-07

The first T2 commit had correct plumbing but weak discipline: it split
provider and logger coverage instead of proving the policy matrix through
the framed path and session DB. The hardening follow-up adds the missing
test surface and rule model:

- Exact `tools/call` tool-name audit rules.
- Exact `resources/read` MCP resource URI audit rules.
- Tool and prompt argument-name audit rules.
- Tool and prompt argument-value audit rules.
- Response/return-value audit rules, including nested result paths.
- Deny-over-allow precedence.
- Live policy mutation on an already-open framed MCP session.
- E2E framed-path DB assertions for request-rule blocks,
  response-rule blocks, and actual legacy policy tool blocks.
- Request-rule denies now short-circuit before aggregator dispatch;
  response-rule denies replace the original result with a sanitized
  policy error before the guest receives it.
- The framed path now re-reads policy per request before dispatch, so
  live policy changes are visible to both the audit provider and the
  borrowed gateway handler.
- The existing gateway env-var tests now serialize their process-global
  `CAPSEM_MCP_INFLIGHT` mutations under a test mutex; a parallel rerun
  caught that old race while verifying this change.

Verification run for the hardening checkpoint:
- `cargo test -p capsem-core mitm_proxy::mcp_frame -- --nocapture`
- `cargo test -p capsem-core mcp::policy -- --nocapture`
- `cargo test -p capsem-core mcp::gateway -- --nocapture`
- `cargo check --workspace --all-targets`
- Session DB sanity query on `mitm-t2-hardening-enforce-bench`: grouped
  `mcp_calls` showed `policy_mode=audit_only`,
  `policy_action=allow`, `policy_rule=mcp.tool.local__echo`, and
  321638 framed benchmark calls.
- scoped `mcp-load` regression: `capsem_exec
  mitm-t2-hardening-enforce-bench 'CAPSEM_MCP_TRANSPORT=framed
  CAPSEM_BENCH_MCP_DURATION=10 capsem-bench mcp-load'` passed with
  0 errors at concurrency 1/10/50/200; rps
  2067.1 / 9865.1 / 10304.1 / 9927.4 and p99
  0.6 / 1.5 / 6.3 / 23.6 ms.

## T3 Update - 2026-05-08

T3 is implemented for the framed MITM path. `MitmProxyConfig` now owns
an `McpEndpointState` instead of borrowing `McpGatewayConfig`; the
framed endpoint dispatches `initialize`, tool/resource/prompt list,
tool calls, resource reads, and prompt gets directly through the
low-privilege `AggregatorClient`. The endpoint owns policy snapshots,
in-flight permits, and method-aware timeout configuration while the
MITM frame layer owns `mcp_calls` writes through the session `DbWriter`.
The aggregator remains DB-free.

Timeout behavior is now explicit: non-`tools/call` methods default to
60s via `CAPSEM_MCP_DEFAULT_TIMEOUT_SECS`; `tools/call` defaults to
300s via `CAPSEM_MCP_TOOL_CALL_TIMEOUT_SECS`; and tool-call catalog
overrides are clamped by `CAPSEM_MCP_TOOL_CALL_TIMEOUT_CEILING_SECS`,
default 300s. A timeout returns a JSON-RPC error and records a terminal
`mcp_calls` row with `decision=error`.

Verification run for the T3 checkpoint:
- `cargo test -p capsem-core mitm_proxy::mcp_frame -- --nocapture`
- `cargo test -p capsem-core mcp::gateway -- --nocapture`
- `cargo test -p capsem-core mcp::policy -- --nocapture`
- `cargo test -p capsem-core mitm_proxy -- --nocapture` (rerun
  escalated after sandboxed integration sockets hit `EPERM`)
- `cargo test -p capsem-core mcp -- --nocapture` (rerun escalated
  after sandboxed live HTTP/MCP calls hit network restrictions)
- `cargo check --workspace --all-targets`
- Aggregator boundary check: `rg -n "DbWriter|McpCall|session.db|WriteOp"
  crates/capsem-core/src/mcp/aggregator.rs crates/capsem-mcp-aggregator
  crates/capsem-core/src/net/mitm_proxy/mcp_frame.rs` showed DB writes
  only in the MITM frame path; the aggregator crate only contains a
  diagnostic comment mentioning `session.db`.
- Session DB sanity query on `mitm-t3-endpoint-bench`: grouped
  `mcp_calls` showed `policy_mode=audit_only`,
  `policy_action=allow`, `policy_rule=mcp.tool.local__echo`, and
  322799 framed benchmark calls.
- scoped `mcp-load` regression: `capsem_exec mitm-t3-endpoint-bench
  'CAPSEM_MCP_TRANSPORT=framed CAPSEM_BENCH_MCP_DURATION=10
  capsem-bench mcp-load'` passed with 0 errors at concurrency
  1/10/50/200; rps 2186.3 / 9984.0 / 10217.6 / 9891.9 and p99
  0.6 / 1.5 / 6.0 / 25.6 ms.

### T3 Coverage Assessment - 2026-05-08

Honest status: T3 is an internal implementation milestone, not a
trust-complete endpoint milestone. It has strong Rust contract coverage
for the framed parser/interpreter, local policy/provider rules, logger
schema, endpoint dispatch helpers, timeout mapping, and `mcp-load`
performance. It does not yet have the functional, adversarial, and VM
E2E matrix that should accompany a security-sensitive MCP transport
change.

What is missing before this path should be considered production-ready:
- Functional tests through the production endpoint boundary for every
  supported MCP method family: `initialize`, `tools/list`,
  `tools/call`, `resources/list`, `resources/read`, `prompts/list`,
  and `prompts/get`, including aggregator error mapping and
  notification handling.
- Adversarial endpoint tests for malformed JSON, oversized payloads,
  corrupted frames, invalid notification ids, stream-id reuse after
  errors, missing required params, timeout errors, and policy-denied
  request/response paths proving dispatch and leak-prevention behavior.
- VM E2E tests that boot a real Capsem session, use the actual guest
  `capsem-mcp-server` framed transport over `vsock:5002`, execute
  `tools/list` and builtin `tools/call`, and query `session.db` for
  `mcp_calls` attribution, policy fields, terminal decisions, and no
  duplicate rows.
- Policy mutation E2E showing an already-open framed MCP connection
  observes live deny/allow changes at the enforcement boundary.
- Concurrent framed calls from distinct process names in a real session,
  proving `mcp_calls.process_name` remains correct under load.
- Timeout E2E using a controllably slow tool/server so both non-tool and
  `tools/call` deadline behavior produce JSON-RPC errors and terminal
  telemetry rows.
- Hot-resume/reconnect E2E for the future T4 cutover, including the
  no-`vsock:5003` assertion and disconnect counter.
- A real boundary test or static dependency guard proving the low-
  privilege aggregator cannot gain `session.db` write access; the T3
  grep is useful evidence, but not a regression test.

Next-sprint rule: do not close T4/T6 by benchmark or Rust unit coverage
alone. The tracker needs named functional, adversarial, E2E/VM,
telemetry, and performance entries before the cutover is called done.

## T3 Hardening Update - 2026-05-08

This follow-up closes the highest-risk coverage debt from the assessment
above without pretending T4/T6 are finished. The framed path now has a
functional endpoint unit suite, a stronger adversarial frame-session
suite, and a real VM E2E suite that boots Capsem, runs the actual guest
`/run/capsem-mcp-server` with `CAPSEM_MCP_TRANSPORT=framed`, and queries
the resulting `session.db`.

Bugs found and fixed during hardening:
- Invalid JSON in an otherwise well-formed MCP frame returned a parse
  error but did not consume the request stream id. A client could reuse
  the same stream id afterward, violating the monotonic stream lifecycle.
  The frame layer now reserves request stream ids before JSON parsing,
  completes them after parse/validation errors, and rejects later reuse.
- `capsem-service` cleared the child process environment but did not
  forward `CAPSEM_HOME` or the framed MCP timeout knobs to
  `capsem-process`. That meant isolated test/user config roots could
  drift between service and process, and timeout E2E would not exercise
  the configured runtime limits. The process env allowlist now forwards
  `CAPSEM_HOME`, `CAPSEM_MCP_DEFAULT_TIMEOUT_SECS`,
  `CAPSEM_MCP_TOOL_CALL_TIMEOUT_SECS`, and
  `CAPSEM_MCP_TOOL_CALL_TIMEOUT_CEILING_SECS`.
- The first VM E2E harness used unsafe shell quoting for embedded Python
  snippets. The tests failed before hitting MCP. The helper now quotes
  the generated `python3 -c` payload with `shlex.quote`.
- The aggregator subprocess carried a stale diagnostic comment that
  referenced `session.db`. The subprocess still had no DB dependency, but
  the new static guard made the boundary explicit and the comment was
  cleaned up so future regressions are unambiguous.

Coverage added:
- Endpoint functional tests for every currently supported method family:
  `initialize`, `tools/list`, `tools/call`, `resources/list`,
  `resources/read`, `prompts/list`, and `prompts/get`, including
  aggregator error mapping and missing-param rejection before dispatch.
- Adversarial framed-session test for stream-id reuse after invalid JSON,
  with an assertion that no `mcp_calls` rows are written for rejected
  parser-level traffic.
- Static aggregator boundary regression proving
  `crates/capsem-mcp-aggregator` stays free of `capsem-logger`,
  `rusqlite`, `DbWriter`, `DbReader`, `WriteOp`, `McpCall`, and
  `session.db` references.
- VM E2E framed-path tests for builtin `local__echo`, configured
  external stdio `fast__ping`, live policy reload on an already-open
  framed connection, concurrent parent-process attribution, and a slow
  external tool timeout. The tests assert JSON-RPC responses and
  `session.db` rows for method, server/tool names, policy fields,
  request/response previews, process names, denial, and terminal errors.

Verification run for the hardening checkpoint:
- `cargo fmt`
- `cargo test -p capsem-service process_env_allowlist_forwards_mcp_timeout_knobs -- --nocapture`
- `cargo test -p capsem-service -- --nocapture`
- `cargo test -p capsem-core net::mitm_proxy::mcp_endpoint -- --nocapture`
- `cargo test -p capsem-core net::mitm_proxy::mcp_frame -- --nocapture`
- `cargo test -p capsem-core mcp::policy -- --nocapture`
- `cargo test -p capsem-core mcp::aggregator -- --nocapture`
- `cargo build -p capsem -p capsem-service -p capsem-process -p capsem-mcp-aggregator -p capsem-mcp-builtin`
- `PYTHONPYCACHEPREFIX=/private/tmp/capsem-pycache python3 -m py_compile tests/capsem-e2e/test_framed_mcp_mitm.py`
- `UV_CACHE_DIR=/private/tmp/capsem-uv-cache PYTHONPYCACHEPREFIX=/private/tmp/capsem-pycache uv run python -m pytest tests/capsem-e2e/test_framed_mcp_mitm.py -v --tb=short -m e2e` (rerun escalated because macOS blocks process enumeration and VM boot from the sandbox)

Remaining honest debt:
- T6 still needs VM E2E for malformed/oversized requests, corrupted-frame
  reconnect behavior, non-`tools/call` timeout through the guest path,
  notification interleaving, post-resume reconnect, `capsem-doctor`
  coverage, and the final no-`vsock:5003` assertion.
- T4 remains the atomic cutover: guest default transport switch,
  reconnect/disconnect counters, legacy 5003 removal, stale terminology
  cleanup, and post-cutover benchmark artifact.

## T4 Update - 2026-05-08

T4 is implemented as the atomic cutover. The guest
`capsem-mcp-server` no longer has a legacy transport toggle: it connects
to `vsock:5002`, sends the existing NUL metadata prefix, then relays
JSON-RPC stdio as bounded MCP frames with per-frame `process_name`.
The host MITM MCP endpoint is now the only guest MCP ingress path.
`crates/capsem-core/src/mcp/gateway.rs`,
`VSOCK_PORT_MCP_GATEWAY`, and the 5003 process dispatch/classification
path are deleted together.

Disconnect behavior is explicit instead of magical. The relay tracks
in-flight JSON-RPC ids by frame stream id, emits one terminal JSON-RPC
transport error per pending request if the framed socket drops, reconnects
for later stdin requests, and documents why it does not replay
`tools/call`: the external tool side effect may already have happened.
The host frame layer now increments `mitm.mcp_disconnects_total` with an
EOF/error reason whenever a framed MCP connection ends.

Bugs found and fixed during T4:
- The first red test proved `capsem-process` still classified legacy
  port 5003 as a deferred MCP connection. The cutover removes that
  classification and leaves 5003 as `Unknown`; the VM E2E also proves a
  guest-side `AF_VSOCK` connect to 5003 fails.
- Extracting the new relay tests exposed a warnings-as-errors issue:
  `FromRawFd` was imported in production after it became test-only. The
  import now lives in the sibling test file.
- `cargo test -p capsem-core --lib mcp` initially failed under the
  sandbox because its live HTTP/MCP integration tests could not reach
  the network. The same suite passed when rerun with network access.
- Running `just inspect-session pearly-sparrow-tmp` exposed two real
  `scripts/check_session.py` bugs: it used Python 3.10 union syntax even
  though the just recipe may run an older system Python, and it looked
  for per-session DBs under `~/.capsem/sessions` instead of the current
  `~/.capsem/run/sessions`. The script now uses `Optional[str]` and the
  correct split between run-session DBs and `~/.capsem/sessions/main.db`.

Verification run for the T4 checkpoint:
- `cargo fmt`
- `cargo test -p capsem-process collect_parks_sni_but_ignores_removed_legacy_mcp_port -- --nocapture`
- `cargo test -p capsem-process vsock::tests -- --nocapture`
- `cargo test -p capsem-proto vsock_port_constants_are_distinct -- --nocapture`
- `cargo test -p capsem-agent --bin capsem-mcp-server -- --nocapture`
- `cargo test -p capsem-core net::mitm_proxy::mcp_frame -- --nocapture`
- `cargo test -p capsem-core net::mitm_proxy::mcp_endpoint -- --nocapture`
- `cargo test -p capsem-core net::mitm_proxy::metrics -- --nocapture`
- `cargo test -p capsem-core net::mitm_proxy::protocol -- --nocapture`
- `cargo test -p capsem-core --lib mcp -- --nocapture` (rerun
  escalated after sandboxed live HTTP/MCP calls hit network restrictions)
- `cargo check --workspace --all-targets`
- `cargo build -p capsem -p capsem-service -p capsem-process -p capsem-mcp-aggregator -p capsem-mcp-builtin`
- `PYTHONPYCACHEPREFIX=/private/tmp/capsem-pycache python3 -m py_compile scripts/check_session.py scripts/doctor_session_test.py guest/artifacts/capsem_bench/mcp_load.py guest/artifacts/capsem_bench/snapshot.py guest/artifacts/diagnostics/test_mcp.py tests/capsem-e2e/test_framed_mcp_mitm.py`
- `UV_CACHE_DIR=/private/tmp/capsem-uv-cache just _pack-initrd` (rerun
  escalated because Docker/Colima access is outside the sandbox)
- `UV_CACHE_DIR=/private/tmp/capsem-uv-cache PYTHONPYCACHEPREFIX=/private/tmp/capsem-pycache uv run python -m pytest tests/capsem-e2e/test_framed_mcp_mitm.py -v --tb=short -m e2e` passed 6 tests, including default transport, policy reload, process attribution, external stdio tool, tool timeout telemetry, and legacy 5003 closed.
- `UV_CACHE_DIR=/private/tmp/capsem-uv-cache just exec "capsem-doctor -k mcp"` passed 91 selected MCP diagnostics with 2 expected skips.
- `UV_CACHE_DIR=/private/tmp/capsem-uv-cache just exec "CAPSEM_BENCH_MCP_DURATION=10 capsem-bench mcp-load && cat /tmp/capsem-benchmark.json"` passed with 0 errors at concurrency 1/10/50/200; rps 2078.1 / 9565.0 / 9644.3 / 9213.1 and p99 0.7 / 1.6 / 6.7 / 28.6 ms.
- `sqlite3 /Users/elie/.capsem/run/sessions/pearly-sparrow-tmp/session.db ...` showed 305008 `mcp_calls` rows with all `policy_mode`, `policy_action`, and `process_name` fields populated; grouped calls were `python3`, `audit_only`, `allow`.
- `just inspect-session pearly-sparrow-tmp -n 1` passed and showed 305008 `mcp_calls` and 16 `fs_events`.
- `git diff --check`

Remaining honest debt:
- T6 is now covered for the T4 guest MCP path; keep any future MCP
  transport changes from weakening the adversarial/E2E matrix.
- T7 still needs the canonical post-cutover benchmark artifact decision.
- Full `just smoke` and `just test` have not been run for this cutover.

## T4 Coverage Review Update - 2026-05-08

The T4 review closed the T6 E2E gap that remained after the atomic
cutover. The full framed MCP VM E2E file now covers normal operation,
policy denial/reload, external stdio tools, concurrent process
attribution, malformed JSON recovery, oversized guest requests,
corrupted-frame recovery on an established framed connection, both
`tools/call` and non-`tools/call` timeout JSON-RPC errors, notification
interleaving, persistent stop/resume reconnect, and legacy 5003 refusal.
The in-VM doctor MCP subset also gained malformed JSON, notification,
and oversized-request checks.

Bugs and test-design issues found during the review:
- `mcp_calls.request_id` only persisted unsigned numeric JSON-RPC ids.
  String request ids were correctly routed back to the client but became
  `NULL` in `session.db`, breaking correlation for perfectly valid MCP
  clients. The framed logger now preserves string, numeric, and null ids
  as text, with Rust and VM E2E regression coverage.
- The first corrupted-frame E2E attempt sent invalid magic as the first
  bytes after metadata. That is a protocol-sniff rejection, not a framed
  interpreter recovery case. The E2E now establishes a valid MCP framed
  stream first, injects a corrupt bounded frame, receives a JSON-RPC
  invalid-frame error, then proves the same connection still accepts a
  valid follow-up request.
- The first non-tool timeout E2E used `tools/list`; that path returns
  the aggregator's already-built catalog and should not block on a slow
  external server. The timeout E2E now uses a slow external
  `resources/read`, which exercises the real non-`tools/call` default
  timeout and verifies terminal `session.db` telemetry.

Verification run for the coverage review:
- `cargo fmt`
- `PYTHONPYCACHEPREFIX=/private/tmp/capsem-pycache python3 -m py_compile tests/capsem-e2e/test_framed_mcp_mitm.py guest/artifacts/diagnostics/test_mcp.py`
- `cargo test -p capsem-core --lib net::mitm_proxy::mcp_frame -- --nocapture`
- `cargo test -p capsem-agent --bin capsem-mcp-server -- --nocapture`
- `cargo build -p capsem -p capsem-service -p capsem-process`
- `UV_CACHE_DIR=/private/tmp/capsem-uv-cache PYTHONPYCACHEPREFIX=/private/tmp/capsem-pycache uv run python -m pytest tests/capsem-e2e/test_framed_mcp_mitm.py -v --tb=short -m e2e` passed 11 tests.
- `UV_CACHE_DIR=/private/tmp/capsem-uv-cache just exec "capsem-doctor -k mcp"` passed 94 MCP diagnostics, 2 expected skips, and 216 deselected tests.
- `cargo check --workspace --all-targets`
- `UV_CACHE_DIR=/private/tmp/capsem-uv-cache just exec "CAPSEM_BENCH_MCP_DURATION=10 capsem-bench mcp-load && cat /tmp/capsem-benchmark.json"` passed with 0 errors at concurrency 1/10/50/200; rps 2133.9 / 9490.1 / 9716.3 / 9074.1 and p99 0.7 / 1.6 / 6.5 / 33.5 ms.
- `sqlite3 /Users/elie/.capsem/run/sessions/witty-griffin-tmp/session.db ...` showed 304147 `mcp_calls` rows with populated `request_id`, `process_name`, `policy_mode=audit_only`, and `policy_action=allow`.
