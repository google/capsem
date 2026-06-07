# Telemetry and Session Findings

Status: completed; transferred to T7 FD08 and owner rows in T3/T6/T8/T10.
T6 implementation items are resolved as of 2026-05-10; real-session
product-path proof remains in T8/T10.

Agent: Hubble (`019e1268-9f19-72e2-a586-3b2512af7d6e`)
T8 reload/telemetry audit agent: Mendel (`019e1342-9fe8-7b81-b5cb-39d3712ef196`)

## Scope

- Old DB compatibility.
- `policy_hook_events` schema.
- Timeline and triage layers.
- MCP/tool correlation SQL.
- Audit correctness.
- Session proof.

## Findings

- [x] [P1] Timeline/triage cannot prove Policy V2 telemetry.
  - Release impact: release claims around DNS policy, hook fallback, audit,
    snapshot, and trace visibility cannot be verified through the tooling users
    will inspect.
  - Paths: `crates/capsem-service/src/main.rs:3144`,
    `crates/capsem-service/src/main.rs:3177`,
    `crates/capsem-service/src/main.rs:2226`,
    `crates/capsem-mcp/src/main.rs:535`.
  - Detail: `capsem_timeline` exposes only `exec,mcp,net,fs,model`; it omits
    `dns_events`, `policy_hook_events`, `audit_events`, and
    `snapshot_events`. Triage reports denied net, MCP errors, and exec
    failures, so DNS blocks and hook failures/fallbacks are invisible.
  - Proof: add timeline/triage tests for DNS, hook, audit, and snapshot
    layers. Agent tried `cargo test -p capsem-service triage_ -- --nocapture`;
    it ran 0 tests. `timeline_` showed 0 lib tests, then local bin runner hit
    codesign failure.
  - Sprint IDs: T6.3, T8.5, T10.5.
  - Resolution: T6 adds dns/hook/audit/snapshot layers, persistent-session
    timeline resolution, schema-aware old-column fallbacks, MCP help/schema
    updates, triage DNS/hook/audit surfaces, and fixture tests covering all
    layers plus NULL-trace retention. Mendel confirmed T8 still needed a real
    product-path `/timeline/{id}` assertion after a Policy V2 decision; T8 now
    adds that assertion to the framed MCP reload E2E, with T10 owning the
    focused VM proof run.

- [x] [P1] `check_session.py` is false-red for old DBs and blind to current
  DB policy fields.
  - Release impact: old valid sessions will look broken, while current Policy
    V2 evidence can go unchecked.
  - Paths: `scripts/check_session.py:19`, `scripts/check_session.py:42`,
    `scripts/check_session.py:153`,
    `tests/capsem-session-lifecycle/test_db_exists.py:7`,
    `tests/capsem-session-lifecycle/test_db_schema.py:8`,
    `crates/capsem-logger/src/schema.rs:595`.
  - Detail: `policy_hook_events` is mandatory for every inspected DB, so
    pre-hook DBs report missing-table errors. Current checks also miss
    `dns_events`, `exec_events`, `audit_events`, `snapshot_events`, and Policy
    V2 columns on `net_events`, `mcp_calls`, and `dns_events`.
  - Proof: old-style DB fixture without hook table must pass compatibility
    mode; current DB fixture must assert all current policy columns.
  - Sprint IDs: T6.1, T6.5, T10.5.
  - Resolution: T6 makes core tables required, current tables informational
    for legacy DBs, previews DNS/exec/audit/snapshot/hook and Policy V2
    fields, and adds old/current fixture tests.

- [x] [P1] MCP/tool correlation SQL is wrong in session tooling.
  - Release impact: session reports can say MCP calls are uncorrelated even
    when the DB contains the relationship.
  - Paths: `scripts/check_session.py:266`,
    `tests/capsem-session-exhaustive/test_cross_table_data.py:21`,
    `tests/capsem-session-exhaustive/test_tool_calls_data.py:41`.
  - Detail: `check_session.py` compares `mc.timestamp >= tc.call_id`, mixing a
    timestamp with a tool call id. Exhaustive FK tests compare
    `tool_responses.call_id` to integer `tool_calls.id` instead of
    `tool_calls.call_id`.
  - Proof: fixture with `tool_calls.mcp_call_id = 1` and matching
    `mcp_calls.id = 1` must report correlation.
  - Sprint IDs: T6.2, T10.5.
  - Resolution: T6 uses `tool_calls.mcp_call_id = mcp_calls.id`, adds
    trace/timestamp fallback, and fixes exhaustive tests to compare
    `tool_responses.call_id` to `tool_calls.call_id`.

- [x] [P1] Hook failure audit rows still look like real hook decisions.
  - Release impact: fallback blocks can be mistaken for valid remote-hook
    decisions unless consumers inspect secondary fields.
  - Paths: `crates/capsem-logger/src/events.rs:383`,
    `crates/capsem-core/src/net/policy_hook.rs:344`,
    `crates/capsem-core/src/net/policy_hook/tests.rs:191`.
  - Detail: logger contract says hook `decision` is `None` for
    transport/schema failures, but `audit_outcome` always writes
    `Some(response.decision)`.
  - Proof: extend `malformed_schema_fails_closed_and_records_error` to assert
    `decision IS NULL`, plus timeout/status/body-cap fallback variants.
  - Sprint IDs: T3.4, T8.5, T10.4.
  - Resolution: T3 corrected fail-closed audit semantics; T6 surfaces hook
    `status`, `fallback`, `error`, and nullable decision values in
    timeline/triage tooling.

- [x] [P2] Audit/session readers drop trace and policy visibility.
  - Release impact: UI/session views cannot show policy mode/action/rule/reason
    even when schema contains them.
  - Paths: `crates/capsem-logger/src/reader.rs:1393`,
    `crates/capsem-logger/src/reader.rs:1417`,
    `frontend/src/lib/sql.ts:137`,
    `crates/capsem-logger/src/schema.rs:95`,
    `crates/capsem-logger/src/schema.rs:237`.
  - Detail: `audit_events` has `trace_id`, but reader queries omit it and set
    `trace_id: None`. Frontend unified tools SQL omits MCP policy fields.
  - Proof: reader tests and frontend SQL tests for trace_id plus
    `policy_mode`, `policy_action`, `policy_rule`, and `policy_reason`.
  - Sprint IDs: T6.3, T6.4, T8.5.
  - Resolution: T6 includes trace/policy fields in session tooling and
    frontend SQL/detail surfaces; frontend check and Vitest coverage passed.

## Tests / Tooling Notes

- Runtime Capsem MCP tool sampling was blocked by the automatic reviewer.
- `uv run pytest ... --collect-only` was blocked by the `uv` cache sandbox with
  escalation rejected; agent did not route around those limits.
