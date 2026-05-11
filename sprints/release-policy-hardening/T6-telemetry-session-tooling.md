# T6: Telemetry and Session Tooling

## Objective

Make session inspection and telemetry prove Policy V2 behavior accurately
across old and new databases. Tools must not false-red older DBs, and the
unified timeline/triage surfaces must expose DNS policy, hook decisions,
audit, and snapshot events by trace where available.

## Status

Implementation complete as of 2026-05-10. Old/core DB compatibility,
current Policy V2 schema checks, MCP/tool correlation, timeline layers,
triage surfaces, frontend policy fields, lifecycle schema checks, and legacy
migration coverage are implemented. Full real-session product-path trace proof
remains a T8/T10 release gate.

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

## Swarm Transfer Tracker

| Source | Priority | Owner task | Required transfer point | Required proof |
|---|---:|---|---|---|
| FD02 docs-release-metadata | P1 | T6.3, T6.4 | Session telemetry docs depend on verified schema/tooling behavior for `mcp_call_id`, `policy_hook_events`, and write ops. | T6 schema/tooling tests pass before T4 docs are finalized. |
| FD04 core-policy-assets | P1 | T6.3 | Hook fallback audit rows must distinguish fallback from real hook decisions. | Session/timeline tests can display fallback error/fallback fields with `decision IS NULL`. |
| FD07 mcp-policy-boundary | P1 | T6.2 | Builtin policy denials may be logged as successful MCP calls. | Decide telemetry contract and test `mcp_calls`/`net_events` source-of-truth behavior. |
| FD07 mcp-policy-boundary | P2 | T6.2, T6.3 | Trace correlation for MCP child processes is accidental and timeline excludes trace-indexed DNS/hook/audit/snapshot layers. | Trace continuity tests cover child MCP logs plus dns/hook/audit/snapshot timeline layers. |
| FD08 telemetry-session | P1 | T6.3 | Timeline/triage cannot prove Policy V2 telemetry because layers omit DNS, hook, audit, and snapshot. | Timeline/triage tests cover DNS, hook, audit, snapshot layers and one NULL-trace row. |
| FD08 telemetry-session | P1 | T6.1, T6.5 | `check_session.py` false-reds old DBs and misses current Policy V2 fields. | Old DB fixture without hook table passes compatibility mode; current fixture asserts all required policy columns. |
| FD08 telemetry-session | P1 | T6.2 | MCP/tool correlation SQL compares timestamp to call id and exhaustive tests join wrong columns. | Fixture with `tool_calls.mcp_call_id = mcp_calls.id` reports correlation. |
| FD08 telemetry-session | P1 | T3.4, T6.3 | Hook failure audit rows still look like real hook decisions. | T3 logger/core tests and T6 timeline output preserve error/fallback semantics. |
| FD08 telemetry-session | P2 | T6.3, T6.4 | Audit/session readers and frontend SQL drop trace and MCP policy fields. | Reader and frontend SQL tests expose trace_id, policy_mode, policy_action, policy_rule, and policy_reason. |

## Task List

### T6.1 Backward-Compatible Check Session

- [x] Make table checks version-aware: required current tables, informational
  old-DB missing tables, and clear missing-current-required errors.
- [x] Treat `policy_hook_events` absence as informational for old DBs.
- [x] Add `dns_events`, `exec_events`, `audit_events`, `snapshot_events`, and
  Policy V2 columns to current schema previews.
- [x] Add Policy V2 fields for `net_events`, `mcp_calls`, and `dns_events` to
  previews and summaries.
- [x] Add a summary section distinguishing optional-old from required-current
  schema gaps.

### T6.2 MCP and Tool Correlation

- [x] Fix primary correlation to use `tc.mcp_call_id = mc.id` where populated.
- [x] Add fallback correlation using `trace_id` plus a real timestamp window.
- [x] Add regression fixtures catching the timestamp-vs-call-id bug.
- [x] Fix exhaustive cross-table comparison to join
  `tool_responses.call_id` to `tool_calls.call_id`.

### T6.3 Timeline and Triage Coverage

- [x] Add `dns` timeline layer.
- [x] Add `hook` timeline layer for `policy_hook_events`.
- [x] Add `audit` timeline layer.
- [x] Add `snapshot` timeline layer.
- [x] Update layer allowlist/help text.
- [x] Make timeline resolve stopped persistent sessions via existing session
  directory resolution helpers, or explicitly document and test running-only
  behavior.
- [x] Include hook failures/fallbacks and DNS denied/error rows in service
  triage.
- [x] Add timeline tests with a fixture DB containing trace-linked exec, mcp,
  net, fs, model, dns, hook, audit, and snapshot rows, plus one NULL trace row
  proving pre-trace events still surface.

### T6.4 Frontend Session Tooling

- [x] Add MCP policy fields to the frontend unified tools/session query.
- [x] Expose policy mode/action/rule/reason in the tools/session UI where
  relevant.
- [x] Ensure UI labels match normalized T3 action naming.

### T6.5 Schema Migration and Lifecycle Tests

- [x] Update session lifecycle table tests to include current tables:
  `dns_events`, `exec_events`, `audit_events`, `snapshot_events`, and
  `policy_hook_events`.
- [x] Update schema tests to assert Policy V2 columns without skipping missing
  tables.
- [x] Add old-schema migration fixtures that manually create pre-hook and
  pre-policy DBs, run migration, then insert/query new tables/columns.
- [x] Add `check_session.py` fixture tests for old DB, current DB with new
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

- [x] `cargo test -p capsem-logger` (98 unit tests + 126 roundtrip tests passed).
- [x] `cargo test -p capsem-core policy_hook -- --nocapture`
  (23 passed; rerun escalated because a policy-hook test binds localhost).
- [x] `cargo test -p capsem-service timeline_ -- --nocapture` (5 passed).
- [x] `cargo test -p capsem-service triage_ -- --nocapture` (1 passed).
- [x] `cargo test -p capsem-mcp timeline_tool_schema -- --nocapture`
  (1 passed).
- [x] `uv run pytest tests/capsem-session-lifecycle/test_db_exists.py tests/capsem-session-lifecycle/test_db_schema.py -q`
  (13 passed).
- [x] `uv run pytest tests/capsem-session tests/capsem-session-exhaustive -q`
  (52 passed, 1 skipped).
- [x] `uv run pytest tests/capsem-session/test_check_session_compat.py -q`
  (2 passed).
- [x] `pnpm -C frontend run check` (0 errors/warnings).
- [x] `pnpm -C frontend test -- src/lib/__tests__/sql-policy-fields.test.ts`
  (Vitest ran the frontend suite: 18 files, 383 tests passed).
- [x] `git diff --check`.

## Exit Criteria

- [x] Old DBs do not show false-red missing hook table errors.
- [x] Current DBs are checked for DNS, audit, snapshot, hook, and Policy V2
  fields.
- [x] MCP/tool correlation query is meaningful.
- [x] Hook, DNS, audit, and snapshot events are visible in the unified timeline.
- [x] Stopped persistent sessions are either supported by timeline or explicitly
  documented as unsupported.
