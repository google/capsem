# Sprint: MITM MCP Unification

## Goal

Move guest-originated MCP traffic onto the MITM inspection path, remove the old guest MCP router on `vsock:5003`, and make the MITM the canonical parser, policy point, and telemetry writer for guest MCP actions.

This sprint does **not** touch `crates/capsem-gateway`; that crate is the TCP-to-service HTTP gateway for UI, tray, and browser access. In this sprint, "old MCP gateway" means `crates/capsem-core/src/mcp/gateway.rs`, the guest MCP router on `vsock:5003`.

## Chosen Direction

Use a custom framed MCP transport over `vsock:5002`, not HTTP/1.1.

The payload inside each frame remains raw MCP JSON-RPC bytes. MCP is JSON-RPC already; replacing the payload with msgpack would add transcoding without removing the need to inspect method, tool, resource, and arguments for policy. The optimized layer is the transport envelope:

```
guest AI agent
  -> guest capsem-mcp-server
  -> Capsem MCP frames over vsock:5002
  -> MITM MCP endpoint
     -> decode frame metadata
     -> parse JSON-RPC payload
     -> enforce local/corp MCP decision
     -> dispatch through AggregatorClient
     -> write mcp_calls
     -> return framed JSON-RPC response
```

HTTP/1.1 is not part of the execution plan. If framed transport hits a real blocker, stop and revise this sprint rather than quietly implementing an HTTP variant.

The aggregator stays low-privilege. It manages external MCP and builtin MCP subprocesses, catalogs, refresh, list-tools state, and shutdown. It must not gain `DbWriter`, session DB access, service IPC access, VM lifecycle access, or broad filesystem access.

## Relationship To Existing MITM T4

This sprint subsumes the relevant parts of `sprints/mitm-redesign/T4-mcp-aware.md`.

Use T4's parser/interpreter plan as the implementation blueprint:

- `JsonRpcParserHook` or equivalent parser: bounded frame payload bytes to JSON-RPC message.
- `McpInterpreterHook` or equivalent interpreter: JSON-RPC request/response to MCP semantic summary.
- MITM-side `mcp_calls` emission.
- `mitm.mcp_methods_total{method}` metric.
- API shaped for future `tool_calls.mcp_call_id` correlation.

T4 originally described passive observation of HTTP-transport MCP. This sprint makes the MITM MCP endpoint the active execution path for guest MCP.

## Mandatory T0: Baseline And Wire Gate

Do not begin the production rewrite until this gate is closed.

Step 0 is a fresh same-hardware baseline of the current `vsock:5003` MCP path. Do not rely on the committed `benchmarks/mcp-load/baseline.json` alone; that file was captured during the MCP concurrency sprint on a specific bench host. Record the new baseline in `tracker.md` before comparing any candidate.

Required measurements:

1. Current `vsock:5003` path, same hardware, same build mode, same benchmark command.
2. Framed JSON-RPC over `vsock:5002`, with multiplexing and per-frame attribution.

Decision rule:

- Pick framed if it works and is comparably simple.
- Reject any candidate that materially regresses the fresh current-5003 baseline. Record the allowed tolerance in `tracker.md` with the measured numbers; do not use stale hardware numbers as the gate.
- If framed is blocked or materially regresses the fresh baseline, pause the sprint and write down the blocker before considering another transport.

Every hot-path functional commit after T0 must run a scoped `mcp-load` regression check. Do not defer all performance discovery to the end of the sprint.

Benchmark artifacts:

- Preserve the pre-change same-hardware baseline as `benchmarks/mcp-load/baseline-pre-mitm-unification.json` or record why the benchmark harness stores it elsewhere.
- After the transport cutover, write the new locked baseline as `benchmarks/mcp-load/baseline-after-mitm-unification.json`.
- Do not overwrite `benchmarks/mcp-load/baseline.json` without explicitly deciding that it should become the new canonical baseline.

## Frame Transport

The `vsock:5002` first-byte sniff grows a `Protocol::McpFrame` branch. MCP frames are internal host-guest traffic and never resolve through DNS or upstream connectors.

Recommended frame shape:

```text
u32 total_frame_len_be
u16 magic
u8 version
u8 header_len
u32 stream_id
u16 flags
u16 process_name_len
u32 payload_len
process_name bytes
payload bytes (raw MCP JSON-RPC)
```

Requirements:

- Use a multiplexed connection with a pending map keyed by `stream_id`.
- `total_frame_len_be` covers every byte after the length prefix. It must be bounded by a constant max frame size before allocation.
- The total length prefix is the resync boundary. Corruption after a valid length prefix is bounded to one frame.
- If the total length is valid but the inner header, magic, version, or length sums are invalid, discard that frame, emit protocol-error telemetry, and continue at the next frame boundary. If a valid `stream_id` can be recovered and a pending request exists, fail that request.
- Invalid total length or an allocation-sized length is a connection-level protocol error: close the connection, fail in-flight requests, and reconnect.
- No sentinel or checksum in v1. Add one later only if real telemetry shows malformed-frame recovery matters more than reconnect simplicity.
- `stream_id` is `u32`, per-connection monotonic, starts at `1`, and reserves `0` for notifications.
- The sender must never reuse an in-flight `stream_id`. Duplicate in-flight ids are programmer/protocol errors and close the connection.
- On `u32` wraparound, stop assigning new request ids, drain in-flight requests if possible, then reconnect before reusing ids.
- Bound in-flight requests with a semaphore in the same spirit as `CAPSEM_MCP_INFLIGHT`.
- Preserve JSON-RPC `id` exactly inside the payload. `stream_id` is transport-only.
- Carry `process_name` per frame. The existing `\0CAPSEM_META:<process_name>\n` prefix may remain for connection diagnostics, but it is not authoritative telemetry once multiplexing exists.
- Bound and sanitize `process_name`; reject or coerce empty/unreasonably long names.
- The MITM uses per-frame process attribution for `net_events` and `mcp_calls`.
- Responses carry the same `stream_id` as the request.
- JSON-RPC notifications use `stream_id=0`, set a notification flag, and do not enter the guest pending map. The host must not send a response frame for a notification.
- On connection loss, every pending `stream_id` receives a JSON-RPC error response on stdout if the original message had a JSON-RPC `id`; notifications get no response.

JSON-RPC has no idempotency token. If a connection drops during `tools/call`, the side effect may already have executed even though the model receives an error. Document that limitation in `crates/capsem-agent/src/mcp_server.rs` and increment `mitm.mcp_inflight_disconnect_total` or equivalent so the failure mode is measurable.

## Decision Provider Shape

Add a small internal decision abstraction now, even if Phase 1 only uses local policy.

Use `Allow` and `Deny` only in v1. Do not add `Warn` to `capsem_logger::Decision` in this sprint. Existing MCP `ToolDecision::Warn` should continue to execute as allowed and may record a matched rule/reason, but it must not introduce a new database decision string until the reader/frontend/dashboard ripple is intentionally handled.

Sketch:

```rust
pub enum DecisionProtocol {
    Http,
    Model,
    Mcp,
    Dns,
}

pub enum DecisionAction {
    Allow,
    Deny,
}

pub struct DecisionRequest {
    pub protocol: DecisionProtocol,
    pub domain: Option<String>,
    pub method: Option<String>,
    pub path: Option<String>,
    pub process_name: Option<String>,
    pub mcp_method: Option<String>,
    pub mcp_server: Option<String>,
    pub mcp_tool: Option<String>,
    pub mcp_resource_uri: Option<String>,
    pub mcp_prompt: Option<String>,
    pub request_preview: Option<String>,
    pub request_hash: Option<String>,
    pub trace_id: Option<String>,
}

pub struct DecisionResult {
    pub action: DecisionAction,
    pub matched_rule: String,
    pub reason: String,
    pub audit_only: bool,
}
```

Future corp providers can plug into the same shape:

- `LocalDecisionProvider`
- `RemoteCorpDecisionProvider`
- `CompositeDecisionProvider`

Remote decision calls must be host-side only. Credentials stay host-side in corp config. Timeouts must be short and explicit. Failure behavior must be corp-configurable: fail-closed or fail-open/audit-only. Prefer request previews and hashes over full bodies.

### Remote Corp Decision Path

The design must allow the MITM hook pipeline to forward MCP decisions to a remote corporate decision system later.

Requirements for that extension point:

- The remote provider is called from the host-side MITM path, never from the guest.
- The request carries structured metadata: protocol, synthetic endpoint, process name, MCP method, server/tool/resource/prompt fields, trace id, request preview, and request hash.
- Full MCP bodies are not sent by default. Corp config may opt into body forwarding separately, but v1 should be preview/hash first.
- Provider timeout and fail behavior are explicit config, not implicit defaults.
- The local provider and remote provider return the same `DecisionResult` shape so telemetry and enforcement do not depend on which provider made the decision.

### Audit-Only Semantics

This sprint adds the telemetry fields needed to make audit-only behavior testable:

- `mcp_calls.policy_mode`: `enforced` or `audit_only`
- `mcp_calls.policy_action`: provider decision, `allow` or `deny`
- `mcp_calls.policy_rule`: matched rule id/name
- `mcp_calls.policy_reason`: human-readable reason

Update `DbWriter`, session readers, and inspect output together with the schema migration. `mcp_calls.decision` remains the effective execution result (`allowed`, `denied`, or `error`); the new policy fields describe what the policy provider said.

Truth table:

| Decision action | `audit_only` | Execute request | Logged decision | Required metadata |
|---|---:|---:|---|---|
| `Allow` | `false` | yes | `allowed` | `policy_mode=enforced`, `policy_action=allow` |
| `Allow` | `true` | yes | `allowed` | `policy_mode=audit_only`, `policy_action=allow` |
| `Deny` | `false` | no | `denied` | `policy_mode=enforced`, `policy_action=deny`, rule/reason required |
| `Deny` | `true` | yes | `allowed` | `policy_mode=audit_only`, `policy_action=deny`, rule/reason required |

Do not invent a `warn` decision string for `Deny + audit_only=true`. Do not silently collapse audit-only denial into indistinguishable allowed telemetry.

### Decision Granularity

Build `DecisionRequest` fields per MCP method:

| MCP method | Decision scope | Required fields |
|---|---|---|
| `initialize` | endpoint/session | `protocol=Mcp`, `mcp_method=initialize`, `process_name`, `trace_id` |
| `notifications/initialized` | endpoint/session | `protocol=Mcp`, `mcp_method=notifications/initialized`, `process_name`, `trace_id` |
| `tools/list` | catalog/server set | `mcp_method=tools/list`, `mcp_server=*`, `request_hash` |
| `tools/call` | server + tool | `mcp_method=tools/call`, parsed `mcp_server`, parsed `mcp_tool`, args preview/hash |
| `resources/list` | catalog/server set | `mcp_method=resources/list`, `mcp_server=*`, `request_hash` |
| `resources/read` | resource URI | `mcp_method=resources/read`, `mcp_resource_uri`, parsed server if namespaced, preview/hash |
| `prompts/list` | catalog/server set | `mcp_method=prompts/list`, `mcp_server=*`, `request_hash` |
| `prompts/get` | server + prompt | `mcp_method=prompts/get`, parsed `mcp_server`, `mcp_prompt`, preview/hash |

Malformed or unknown methods still emit telemetry with `mcp_method` populated from the raw method string when possible.

## Logging Ownership

- MITM request telemetry writes `net_events`.
- MITM MCP endpoint/interpreter writes `mcp_calls`.
- Aggregator writes no session DB rows.
- Delete or disable gateway-side `log_mcp_call`.
- Do not produce duplicate `mcp_calls`.

`mcp_calls.trace_id` must be non-null for guest MCP actions. `mcp_calls.process_name` must come from per-frame attribution, not a pooled connection-level default.

For framed MCP transport, `net_events` uses a synthetic internal endpoint identity such as `mcp.capsem.internal` with path `/jsonrpc` or method-derived paths. That identity is never DNS-resolved or forwarded upstream; it exists only for policy, inspection, and telemetry consistency.

The aggregator may emit structured stderr tracing for its own subprocess lifecycle, refresh, tool catalog, and shutdown events. Those traces are diagnostics only and must not be session DB writes.

## Guest Reproxy

Rewrite `crates/capsem-agent/src/mcp_server.rs` around the framed transport.

Requirements:

- Read JSON-RPC lines from stdin.
- Write MCP frames to `vsock:5002`.
- Carry process attribution in each frame.
- Support concurrent requests from one agent.
- Use one stdout writer so responses never interleave.
- Notifications return no stdout line.
- On disconnect with in-flight requests, surface JSON-RPC errors for those pending requests and reconnect for later requests.
- Document the `tools/call` idempotency limitation described above.
- Add disconnect metrics.
- Add tests for concurrent requests from distinct guest processes so `mcp_calls.process_name` cannot drift.

## MITM MCP Endpoint

Create a focused module, likely:

```
crates/capsem-core/src/net/mitm_proxy/mcp_endpoint.rs
```

Responsibilities:

- Decode MCP frames and bound frame/body sizes.
- Parse bounded JSON-RPC requests.
- Implement MCP methods:
  - `initialize`
  - `notifications/initialized`
  - `tools/list`
  - `tools/call`
  - `resources/list`
  - `resources/read`
  - `prompts/list`
  - `prompts/get`
- Build a method-specific `DecisionRequest`.
- Apply MCP/local/corp decision before dispatch.
- Enforce method-aware MCP request timeouts.
- Dispatch through `AggregatorClient`.
- Build JSON-RPC responses.
- Emit one `mcp_calls` row per request with a response or terminal error.
- Return no body for notifications.
- Increment `mitm.mcp_methods_total{method}`.

Timeouts:

- Non-`tools/call` methods default to `60s` via `CAPSEM_MCP_DEFAULT_TIMEOUT_SECS`.
- `tools/call` defaults to `300s` via `CAPSEM_MCP_TOOL_CALL_TIMEOUT_SECS`.
- Per-tool timeout overrides may come from the aggregator catalog when available.
- `CAPSEM_MCP_TOOL_CALL_TIMEOUT_CEILING_SECS` defaults to `300s`; catalog overrides cannot exceed the ceiling.
- Timeout produces a JSON-RPC error and a terminal `mcp_calls` row with `decision=error`.

State belongs in MITM config, not the aggregator:

```rust
pub struct McpTimeouts {
    pub default_timeout: std::time::Duration,      // 60s default
    pub tool_call_default: std::time::Duration,    // 300s default
    pub tool_call_ceiling: std::time::Duration,    // 300s default
}

pub struct McpEndpointState {
    pub aggregator: AggregatorClient,
    pub policy: tokio::sync::RwLock<Arc<McpPolicy>>,
    pub inflight: Arc<tokio::sync::Semaphore>,
    pub timeouts: McpTimeouts,
}
```

`MitmProxyConfig` may hold:

```rust
pub mcp_endpoint: Option<Arc<McpEndpointState>>
```

Use the existing MITM `DbWriter` for `mcp_calls`.

## Simplifications

Delete:

- `crates/capsem-core/src/mcp/gateway.rs`
- `McpGatewayConfig`
- `serve_mcp_session`
- old gateway-side `log_mcp_call`
- `VSOCK_PORT_MCP_GATEWAY = 5003`
- 5003 dispatch in `crates/capsem-process/src/vsock.rs`
- 5003 port classification and tests
- docs/comments that describe guest MCP as a "gateway"

Keep:

- `crates/capsem-core/src/mcp/aggregator.rs`
- `crates/capsem-core/src/mcp/policy.rs`
- `crates/capsem-core/src/mcp/types.rs`
- `crates/capsem-core/src/mcp/server_manager.rs`
- `crates/capsem-mcp-aggregator`
- `crates/capsem-mcp-builtin`

Update terminology:

- `capsem-gateway`: only the TCP-to-service HTTP gateway.
- `MITM MCP endpoint`: guest MCP path through `vsock:5002`.
- Avoid "MCP gateway" for new code.

## Builtin And External MCP Outbound Gap

Phase 1 does not route aggregator or builtin outbound `reqwest` traffic through MITM.

Do not delete builtin HTTP domain policy or direct builtin `net_events` emission in Phase 1.

Add tests that explicitly document the bounded Phase 1 gap:

- Builtin HTTP tools still enforce configured domain policy.
- Builtin HTTP tools still emit their existing `net_events`.
- External MCP server outbound HTTP remains host-side and not MITM-audited in Phase 1; add a named gap test or documented test fixture so Phase 2 has something concrete to delete or invert.

Phase 2 will route aggregator/builtin outbound HTTP through MITM and then remove duplicate builtin network logging.

## Resume And Disconnect Behavior

The transport must handle Apple VZ post-restore reconnect patterns.

Required behavior:

- After VM suspend/resume, stale framed connections reconnect.
- New MCP requests succeed after reconnect.
- In-flight requests on a broken connection receive explicit JSON-RPC errors.
- The process does not hang waiting for a response that can never arrive.
- `capsem-doctor` MCP diagnostics still pass after the transport cutover; update tests that assume `vsock:5003`.

## Files Likely Touched

- `crates/capsem-agent/src/mcp_server.rs`
- `crates/capsem-agent/src/mcp_server/tests.rs` if tests are extracted
- `crates/capsem-core/src/net/mitm_proxy/mod.rs`
- `crates/capsem-core/src/net/mitm_proxy/protocol.rs`
- `crates/capsem-core/src/net/mitm_proxy/mcp_endpoint.rs`
- `crates/capsem-core/src/net/parsers/jsonrpc_parser.rs`
- `crates/capsem-core/src/net/interpreters/mcp_interpreter.rs`
- `crates/capsem-core/src/net/mitm_proxy/events.rs`
- `crates/capsem-core/src/net/mitm_proxy/metrics.rs`
- `crates/capsem-core/src/mcp/mod.rs`
- `crates/capsem-core/src/mcp/aggregator.rs`
- `crates/capsem-process/src/main.rs`
- `crates/capsem-process/src/vsock.rs`
- `crates/capsem-proto/src/lib.rs`
- `crates/capsem-proto/src/tests.rs`
- `crates/capsem-core/src/vm/boot.rs`
- `crates/capsem-core/src/vm/registry.rs`
- `guest/artifacts/capsem-doctor/` MCP diagnostics if they reference 5003
- `benchmarks/mcp-load/`
- docs under `docs/src/content/docs/architecture/`
- tests under `tests/capsem-mcp/` and `tests/capsem-session/`

## Verification

Unit and targeted:

```bash
cargo test -p capsem-core mitm_proxy
cargo test -p capsem-core mcp
cargo test -p capsem-agent mcp_server
cargo test -p capsem-proto
just test-mcp
```

Runtime:

- Guest `tools/list` works.
- Guest builtin `tools/call` works.
- External MCP tool works when configured.
- Notifications interleaved with concurrent `tools/call` do not deadlock.
- `net_events` contains the MCP transport request with correct per-frame `process_name`.
- `mcp_calls` contains one row per MCP request, written by MITM.
- `mcp_calls.process_name` is correct for concurrent requests from N distinct guest processes.
- No duplicate `mcp_calls`.
- Aggregator has no `DbWriter`.
- `rg VSOCK_PORT_MCP_GATEWAY crates/` returns nothing.
- `vsock:5003` connection attempt fails.
- `capsem-doctor` MCP tests pass post-cutover.

Adversarial:

- Corp/local policy denies the MCP endpoint: request fails and telemetry records deny.
- Malformed JSON returns JSON-RPC parse error.
- Oversized request is rejected.
- Non-`tools/call` timeout at 60s and `tools/call` timeout/ceiling behavior return JSON-RPC errors and record terminal telemetry.
- Builtin Phase 1 outbound gap tests pass.

Resume:

- Create a persistent VM, start guest MCP traffic, suspend/resume, verify new MCP requests succeed and stale in-flight requests do not hang.

Performance:

- Re-baseline current `5003` on the same hardware before production changes.
- Run `mcp-load` after each hot-path commit.
- Record rps and p99 in `tracker.md`.
- Write pre/post baseline JSON artifacts as described in T0.

Full gate:

```bash
just smoke
just test
```

If telemetry changed, also verify with:

```bash
just inspect-session
```

## Commit Shape

Expected functional milestones:

1. `sprint(mitm): baseline MCP transport for MITM endpoint` - fresh current-5003 baseline, framed spike result, tracker update.
2. `feat(mitm): add framed MCP parser and interpreter` - protocol sniff, frame codec, parser/interpreter/tests/metrics, scoped `mcp-load`.
3. `feat(mitm): execute guest MCP through MITM endpoint` - endpoint, decision provider, request timeout, mcp_calls, aggregator dispatch, scoped `mcp-load`.
4. `feat(mcp): cut guest MCP over to MITM frames and remove 5003` - atomic release unit containing guest relay rewrite plus deletion of old 5003 router/constants/docs/tests, scoped `mcp-load`.
5. `test(mcp): add MCP MITM e2e, gap, doctor, resume, and load coverage` - integration and adversarial tests plus new baseline artifact.

Commit 4 must not be split into separately releasable guest-agent and host-router commits. The initrd guest binary and host process must agree on the transport.

Each commit updates `CHANGELOG.md` under `## [Unreleased]`, stages files explicitly, and passes its scoped tests plus the hot-path benchmark gate when it touches MCP transport performance.
