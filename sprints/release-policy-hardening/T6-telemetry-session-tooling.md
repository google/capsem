# T6: Telemetry and Session Tooling

## Objective

Make session inspection and telemetry prove Policy V2 behavior accurately
across old and new databases. Tools must not false-red older DBs, and the
unified timeline/triage surfaces must expose DNS policy, hook decisions,
audit, and snapshot events by trace where available.

## Owned Files

- `scripts/check_session.py`
- `crates/capsem-service/src/main.rs`
- `crates/capsem-service/src/triage.rs`
- `crates/capsem-mcp/src/main.rs`
- `crates/capsem-logger/src/schema.rs`
- `crates/capsem-logger/src/events.rs`
- `crates/capsem-logger/src/reader.rs`
- `frontend/src/lib/sql.ts`
- `tests/capsem-session-lifecycle/*`
- `tests/capsem-session/*`
- `tests/capsem-session-exhaustive/*`
- `docs/src/content/docs/architecture/session-telemetry.md`

## Findings

- [P2] `check_session.py` treats `policy_hook_events` as mandatory for every
  DB; older DBs created before hook telemetry will show false-red missing table
  warnings.
- [P2] `check_session.py` omits `dns_events`, `exec_events`, `audit_events`,
  `snapshot_events`, and Policy V2 columns from table/preview checks.
- [P2] `check_session.py` correlates MCP rows with `mc.timestamp >=
  tc.call_id`, comparing a timestamp string to a tool call id.
- [P2] `capsem_timeline` allowlists only `exec,mcp,net,fs,model`; schema now
  has traceable `dns_events`, `policy_hook_events`, `audit_events`, and
  `snapshot_events`.
- [P2] `capsem_timeline` reads only running `state.instances`, so stopped
  persistent sessions are excluded.
- [P2] Triage omits DNS policy blocks and hook failures/fallbacks.
- [P2] Frontend tools/session SQL omits MCP policy fields from the unified tools
  table.
- [P2] Session lifecycle schema tests expect an older table set and can skip
  missing tables instead of failing on missing current columns.
- [P3] Logger migration tests start from current `create_tables()`, so they do
  not exercise old DBs missing hook or Policy V2 columns.
- [P3] Exhaustive cross-table test compares `tool_responses.call_id` against
  `tool_calls.id`, not `tool_calls.call_id`.

## Task List

### T6.1 Backward-Compatible Check Session

- [ ] Make table checks version-aware: required current tables, informational
  old-DB missing tables, and clear missing-current-required errors.
- [ ] Treat `policy_hook_events` absence as informational for old DBs.
- [ ] Add `dns_events`, `exec_events`, `audit_events`, `snapshot_events`, and
  Policy V2 columns to current schema previews.
- [ ] Add Policy V2 fields for `net_events`, `mcp_calls`, and `dns_events` to
  previews and summaries.
- [ ] Add a summary section distinguishing optional-old from required-current
  schema gaps.

### T6.2 MCP and Tool Correlation

- [ ] Fix primary correlation to use `tc.mcp_call_id = mc.id` where populated.
- [ ] Add fallback correlation using `trace_id` plus a real timestamp window.
- [ ] Add regression fixtures catching the timestamp-vs-call-id bug.
- [ ] Fix exhaustive cross-table comparison to join
  `tool_responses.call_id` to `tool_calls.call_id`.

### T6.3 Timeline and Triage Coverage

- [ ] Add `dns` timeline layer.
- [ ] Add `hook` timeline layer for `policy_hook_events`.
- [ ] Add `audit` timeline layer.
- [ ] Add `snapshot` timeline layer.
- [ ] Update layer allowlist/help text.
- [ ] Make timeline resolve stopped persistent sessions via existing session
  directory resolution helpers, or explicitly document and test running-only
  behavior.
- [ ] Include hook failures/fallbacks and DNS denied/error rows in service
  triage.
- [ ] Add timeline tests with a fixture DB containing trace-linked exec, mcp,
  net, fs, model, dns, hook, audit, and snapshot rows, plus one NULL trace row
  proving pre-trace events still surface.

### T6.4 Frontend Session Tooling

- [ ] Add MCP policy fields to the frontend unified tools/session query.
- [ ] Expose policy mode/action/rule/reason in the tools/session UI where
  relevant.
- [ ] Ensure UI labels match normalized T3 action naming.

### T6.5 Schema Migration and Lifecycle Tests

- [ ] Update session lifecycle table tests to include current tables:
  `dns_events`, `exec_events`, `audit_events`, `snapshot_events`, and
  `policy_hook_events`.
- [ ] Update schema tests to assert Policy V2 columns without skipping missing
  tables.
- [ ] Add old-schema migration fixtures that manually create pre-hook and
  pre-policy DBs, run migration, then insert/query new tables/columns.
- [ ] Add `check_session.py` fixture tests for old DB, current DB with new
  tables, and MCP correlation.

## Proof Matrix

| Category | Required proof |
|---|---|
| Unit/contract | logger migration from old schema creates hook and Policy V2 fields. |
| Functional | `check_session.py` reports old DBs cleanly and current DBs strictly. |
| Telemetry | timeline returns dns/hook/audit/snapshot events by trace/layer. |
| E2E/session | session lifecycle/exhaustive tests assert current tables and audit columns. |
| UI | frontend session/tool query exposes MCP policy fields. |

## Verification

- [ ] `cargo test -p capsem-logger`
- [ ] `cargo test -p capsem-core policy_hook -- --nocapture`
- [ ] `cargo test -p capsem-service timeline_ -- --nocapture`
- [ ] `cargo test -p capsem-service triage_ -- --nocapture`
- [ ] `cargo test -p capsem-mcp timeline_tool_schema -- --nocapture`
- [ ] `uv run pytest tests/capsem-session-lifecycle/test_db_exists.py tests/capsem-session-lifecycle/test_db_schema.py -q`
- [ ] `uv run pytest tests/capsem-session tests/capsem-session-exhaustive -q`
- [ ] `uv run pytest tests/capsem-session/test_check_session_compat.py -q`

## Exit Criteria

- [ ] Old DBs do not show false-red missing hook table errors.
- [ ] Current DBs are checked for DNS, audit, snapshot, hook, and Policy V2
  fields.
- [ ] MCP/tool correlation query is meaningful.
- [ ] Hook, DNS, audit, and snapshot events are visible in the unified timeline.
- [ ] Stopped persistent sessions are either supported by timeline or explicitly
  documented as unsupported.
