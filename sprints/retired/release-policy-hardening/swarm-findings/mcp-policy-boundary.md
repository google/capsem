# MCP Policy Boundary Findings

Status: completed; transferred to T7 FD07 and owner rows in T3/T5/T6/T8/T10.
T3 notification/telemetry, T5 service/process, and T6 telemetry/tooling items
are implemented where marked; T8/T10 runtime E2E proof remains open.

Agent: Chandrasekhar (`019e1268-9d79-7cf1-bae8-7581987836b8`)

## Scope

- MCP notification bypass.
- External server env leakage.
- Builtin tool policy and telemetry.
- Gateway auth.
- Helper packaging contracts.
- Trace correlation.

## Findings

- [x] [P0] MCP notification bypass can execute tools without policy blocking
  or `mcp_calls` telemetry.
  - Release impact: a no-id `tools/call` can bypass request policy, leak
    arguments, and execute through the aggregator without the expected audit
    trail.
  - Paths: `crates/capsem-core/src/net/mitm_proxy/mcp_frame.rs:156`,
    `crates/capsem-core/src/net/mitm_proxy/mcp_frame.rs:1391`,
    `crates/capsem-core/src/net/mitm_proxy/mcp_endpoint.rs:119`,
    `crates/capsem-core/src/net/mitm_proxy/mcp_endpoint.rs:179`,
    `crates/capsem-agent/src/mcp_server.rs:316`.
  - Detail: `handle_framed_mcp` computes policy, then returns early for
    notifications before block/log handling. Validator only checks
    notification has no id, not allowed notification methods. Endpoint will
    dispatch no-id `tools/call`; guest framing classifies any JSON-RPC object
    without `id` as notification.
  - Proof: adversarial no-id `tools/call` notification test proving aggregator
    dispatch is zero, sensitive args absent from logs, and denied/audit row
    exists.
  - Run: `cargo test -p capsem-core mcp_frame -- --nocapture`.
  - Sprint IDs: T3.5, T8.5, T10.4.

- [ ] [P0] Linux release packaging omits required MCP helper binaries.
  - Release impact: installed Linux release can lose MCP discovery/tools at
    runtime while process falls back to an empty stub.
  - Paths: `crates/capsem-process/src/main.rs:725`,
    `crates/capsem-process/src/main.rs:730`,
    `crates/capsem-process/src/main.rs:330`,
    `.github/workflows/release.yaml:516`, `scripts/repack-deb.sh:34`,
    `scripts/deb-postinst.sh:34`, `scripts/simulate-install.sh:42`,
    `tests/test_repack_deb.py:27`,
    `tests/capsem-install/conftest.py:89`.
  - Proof: update Linux install contract to eight binaries and verify `.deb`
    contents plus installed layout.
  - Sprint IDs: T5.1, T10.1, T10.5.

- [ ] [P1] External stdio MCP servers inherit parent environment, leaking
  config/session paths and possibly secrets.
  - Release impact: external tools can see host/session env that should not be
    delegated.
  - Paths: `crates/capsem-process/src/main.rs:800`,
    `crates/capsem-core/src/mcp/server_manager.rs:324`,
    `crates/capsem-core/src/mcp/aggregator.rs:491`.
  - Detail: process spawns aggregator without `env_clear()`; aggregator/server
    manager spawns stdio servers without `env_clear()`, only adding configured
    env on top. Existing safety test only proves aggregator source is
    session-DB-free, not env-isolated.
  - Proof: stdio MCP fixture that reports env; assert only explicit allowlist,
    configured vars, and trace vars are visible, not `CAPSEM_USER_CONFIG`,
    `CAPSEM_CORP_CONFIG`, `CAPSEM_HOME`, or sentinel secrets.
  - Sprint IDs: T5.3, T10.5.

- [ ] [P1] Builtin HTTP tools check domain policy only before redirects.
  - Release impact: a request to an allowed domain can follow a redirect to a
    blocked host.
  - Paths: `crates/capsem-mcp-builtin/src/main.rs:513`,
    `crates/capsem-core/src/mcp/builtin_tools.rs:269`,
    `crates/capsem-core/src/mcp/builtin_tools.rs:305`,
    `crates/capsem-core/src/mcp/builtin_tools.rs:393`,
    `crates/capsem-core/src/mcp/builtin_tools.rs:443`,
    `crates/capsem-core/src/mcp/builtin_tools.rs:552`,
    `crates/capsem-core/src/mcp/builtin_tools.rs:585`.
  - Proof: local redirect from allowed host to blocked host; assert blocked
    final host is never fetched and telemetry records denial for final host.
  - Sprint IDs: T5.5, T8.5, T10.5.

- [ ] [P1] `McpRefreshTools` drops builtin MCP server from live sessions.
  - Release impact: refreshing tools in a running VM can remove local builtin
    tools and lose updated domain policy wiring.
  - Paths: `crates/capsem-process/src/main.rs:345`,
    `crates/capsem-process/src/ipc.rs:619`,
    `tests/capsem-e2e/test_framed_mcp_mitm.py:528`.
  - Detail: initial boot uses `build_server_list_with_builtin`; refresh uses
    plain `build_server_list`.
  - Proof: refresh a running VM and assert `local__echo`/`local__http_headers`
    remain available and honor updated domain policy.
  - Sprint IDs: T5.5, T8.4, T10.5.

- [ ] [P1] Builtin policy denials are logged as successful MCP calls.
  - Release impact: timeline/UI can show an MCP call as allowed while the
    underlying net event was denied.
  - Paths: `crates/capsem-core/src/net/mitm_proxy/mcp_frame.rs:661`,
    `crates/capsem-mcp-builtin/src/main.rs:392`,
    `tests/capsem-e2e/test_framed_mcp_mitm.py:886`.
  - Detail: `log_mcp_call_with_policy` marks any response without JSON-RPC
    error as `allowed`; builtin logical failures become MCP tool `isError`
    results rather than JSON-RPC errors.
  - Proof: decide telemetry contract; either `mcp_calls` must show denied/error
    with policy fields, or UI/timeline must explicitly treat `net_events` as
    source of truth.
  - Sprint IDs: T6.2, T8.5, T10.5.

- [x] [P2] MCP Policy V2 telemetry still says `audit_only`/`deny` for enforced
  blocks.
  - Release impact: consumers cannot distinguish enforcement from audit-only
    behavior without private translation knowledge.
  - Paths: `crates/capsem-core/src/net/mitm_proxy/mcp_frame.rs:575`,
    `crates/capsem-core/src/net/mitm_proxy/mcp_frame.rs:740`,
    `crates/capsem-core/src/net/mitm_proxy/mcp_frame.rs:942`,
    `tests/capsem-e2e/test_framed_mcp_mitm.py:689`.
  - Proof: normalize enforced telemetry to `enforce`/`block`, or document the
    translation boundary and update consumers/tests.
  - Run: `cargo test -p capsem-core mcp_frame -- --nocapture`.
  - Sprint IDs: T3.6, T8.5, T10.4.

- [ ] [P2] Trace correlation for MCP child processes is accidental, not a
  hardened contract.
  - Release impact: env isolation fixes could silently break trace continuity.
  - Paths: `crates/capsem-process/src/main.rs:805`,
    `crates/capsem-core/src/mcp/server_manager.rs:324`,
    `crates/capsem-service/src/main.rs:3177`,
    `crates/capsem-logger/src/schema.rs:220`,
    `crates/capsem-logger/src/schema.rs:495`.
  - Detail: aggregator receives explicit trace env from process, but stdio
    children inherit trace only because broader env is inherited. Timeline also
    excludes trace-indexed DNS, hook, audit, and snapshot layers.
  - Proof: after env isolation, explicitly pass trace env to builtin/external
    MCP children and assert `mcp_calls`, `net_events`, and child logs/timeline
    share trace.
  - Sprint IDs: T5.3, T6.2, T6.3, T10.5.

- [ ] [P3] Gateway auth looks structurally protected, but route-matrix proof is
  thin.
  - Release impact: auth coverage might miss new `/policy-hook/spec` and
    fallback proxy routes.
  - Paths: `crates/capsem-gateway/src/main.rs:142`,
    `crates/capsem-gateway/src/main.rs:166`,
    `crates/capsem-gateway/src/auth.rs:151`,
    `crates/capsem-gateway/src/auth/tests.rs:286`.
  - Proof: gateway integration matrix showing `/policy-hook/spec` and service
    fallback routes are 401 without token and proxy only with token.
  - Sprint IDs: T5.2, T10.5.

## T3 Execution Audit, 2026-05-10

Agent: Socrates (`019e12fd-d81b-72d3-a023-e618a6c2edb6`)

Status: completed; no edits made by the agent. Findings captured during T3
implementation.

- [x] T3.5 confirmed no-id `tools/call` notifications could dispatch before
  policy/log handling; fixed with a notification allowlist, denied/audited
  handling for disallowed notifications, zero aggregator dispatch, and
  sanitized request previews.
- [x] T3.6 confirmed Policy V2 MCP telemetry used `audit_only` for enforced
  decisions; fixed with explicit `enforce` mode for Policy V2 matches.
- [x] T3.6 confirmed Policy V2 block telemetry used `deny`; fixed so
  `policy_action = block` while coarse `mcp_calls.decision` remains
  `denied`.
- [x] T3.6 confirmed matched Policy V2 allow rules were dropped before
  telemetry; fixed so allow decisions are preserved unless superseded by a
  higher-priority denial.
- [x] Logger storage was confirmed pass-through; semantic fixes landed in
  `mcp_frame`, with logger persistence verified separately.

## Tests Run

- `cargo test -p capsem-core mcp_frame -- --nocapture` (50 passed).
- `cargo test -p capsem-core mcp_endpoint -- --nocapture` (9 passed).
- `cargo test -p capsem-logger mcp_call -- --nocapture` (15 passed).

## T3 Tests Not Run

- `pytest tests/capsem-e2e/test_framed_mcp_mitm.py -k "policy_v2 or notification" -v`
  remains a T8/T10 VM/E2E proof item.

## Tests Not Run

- Original Chandrasekhar investigation was read-only; no tests were run in that
  pass.
