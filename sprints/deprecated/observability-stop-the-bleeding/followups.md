# Sprint follow-ups: shortcuts I took during the original observability sprint

Each is a corner I cut. Listed roughly by severity / blast radius. The
sprint is "done" only when these are closed too.

## Functional gaps

- [x] **F1. W6 writer-side population.** Schema columns exist on all 7
  tables (mcp_calls, net_events, fs_events, snapshot_events, tool_calls,
  tool_responses, audit_events). NEW rows still write `trace_id = NULL`
  because the writer doesn't pull from ambient context. Plumb
  `capsem_core::telemetry::ambient_capsem_trace_id()` (or an equivalent
  thread-local set on each handler entry) into every event constructor
  in mitm_proxy.rs, gateway.rs, fs_monitor.rs, auto_snapshot scheduler.
  Without this, `capsem_timeline --trace-id X` only joins
  exec_events + model_calls correctly; the other five tables are NULL
  and only show up via the OR-NULL fallback.

- [x] **F2. T4 `dump_frontend_logs` Tauri command missing.** Frontend
  `__capsemDebug.dumpLogs()` invokes a Rust command that doesn't exist.
  Returns an error string at runtime today. Add the IPC handler in
  `crates/capsem-app/src/main.rs` so the dev console can surface the
  current jsonl path.

- [x] **F3. T5 `recordWsEvent` never called.** `frontend/src/lib/tauri-log.ts`
  exports the helper but no caller wires it up. `__capsemDebug.lastWsEvents`
  is always empty. Wire it in `frontend/src/lib/api.ts` WS onmessage
  handler.

- [x] **F4. W4 `#[instrument]` coverage incomplete.** Plan called for
  spans on `with_quiescence`, `save_state`, `restore_state`, `pause`,
  `attach_disk`, `attach_virtiofs_share`, `wait_for_vm_ready`, MCP tool
  dispatch. Shipped: explicit timing on the suspend block + `#[instrument]`
  on `send_ipc_command`, `handle_suspend`, `handle_json_rpc`. MISSING:
  `restore_state`, `wait_for_vm_ready`, `attach_disk`, `attach_virtiofs_share`,
  `pause` (standalone, not inside the suspend block).

- [x] **F5. W4 unhandled-arm coverage incomplete.** Plan listed 5 sites;
  shipped 3 (lifecycle port, handle_guest_msg, vsock dispatch). Convert
  the remaining two: `capsem-mcp-aggregator/src/main.rs:106-108` and
  `capsem-process/src/ipc.rs:306-308`.

- [x] **F6. T2 session-DB cross-reference is stubbed.** `/triage`
  endpoint accepts `id` but ignores it. Plan called for SQL on
  net_events (denied/5xx), mcp_calls (denied/error), exec_events
  (failures) when `id` is set. Wire it through.

- [x] **F7. T3 timeline does no joins.** UNION ALL across 5 tables is
  fine, but plan called for joining `tool_calls.mcp_call_id` -> `mcp_calls.id`
  so a model_call's tool_use rows show their MCP servicing call inline.
  Add the join, ship as a `&joins=mcp` query flag if the existing
  shape needs to stay backwards-compatible.

- [x] **F8. T1 size cap.** Plan called for `--max-bytes` (default 50MB).
  Bundle has no cap today; a 4GB session.db produces a 4GB tar.gz. Add
  the flag + truncate via `VACUUM INTO` for session.db.

- [x] **F9. T2 panic frame parser is fragile.** Heuristic for
  "consecutive `   at <path>:line` lines after a `thread '...' panicked at`
  header" breaks on inlined frames, multi-line panic messages, and
  panics in spawned threads. Add fixture tests for those shapes; tighten
  the parser or accept partial captures gracefully.

- [x] **F10. T1 redactor is ad-hoc.** Five rules. No fuzz, no
  adversarial-shape coverage (Bearer token spanning newlines, base64
  envelope, OPENAI_API_KEY=`sk-proj-...` form that may not match
  `sk-[A-Za-z0-9_-]{20,}` literally). Add adversarial fixtures + maybe
  switch to a mature redactor crate.

## Test gaps

- [x] **T1. W3 protocol-handshake regression test.** Plan called for
  `tests/capsem-service/test_protocol_handshake.py` -- synthetic v0
  client sends raw bincode, asserts service emits structured handshake
  error within 1s. SKIPPED. Unit tests use `socketpair`, not the real
  UDS path. The whole point of W3 was "the today-2026-05-02 silent IPC
  bug fails loudly now" and there's no test that proves it.

- [x] **T2. `just test` never run during sprint.** Cross-compile checks,
  frontend vitest, Python pytest, the full release-shaped pipeline.
  Run it now, fix anything red.

- [x] **T3. `just smoke` never run during sprint.** Real-VM boot path
  exercises every IPC channel my W3/W4/W5 work touched. Compile-clean +
  unit-test-green is not the same as "the wire still works".

- [x] **T4. T4 doctor-bundle path discovery untested.** Host reads
  `<session_dir>/guest/doctor-bundle.tar` then `<session_dir>/workspace/...`.
  Real virtiofs mount path may differ per platform / image / config.
  Verify against a real VM run.

## Documentation gaps

- [x] **D1. Skill updates queue is empty.** Tracker has 6 pending skill
  edits: dev-rust-patterns (try_send! + handshake), dev-installation
  (gateway.log/tray.log + support-bundle + doctor --bundle),
  dev-debugging (schema_hash mismatch first; triage workflow),
  dev-session-debug (every table has trace_id; trace_id host->guest),
  dev-mcp (4 new tools), dev-testing (CI artifact retrieval +
  just test-artifacts). NONE done. Future agent loads `/dev-mcp` and
  doesn't know about `capsem_timeline`/`capsem_panics`/`capsem_triage`/
  `capsem_host_logs`.

- [x] **D2. W5 wire changes undocumented externally.** Optional
  `traceparent` on BootConfig and `_meta` on JSON-RPC are wire shapes
  third parties might build against. No update to docs/ site, no
  update to `references/mcp-wire.md`. Document the new optional fields
  + their semantics.

- [x] **D3. T1 manifest schema has no validator.** I declared
  `schema_version: 1` but wrote no JSON Schema, no migration test,
  nothing that asserts an old-version bundle still parses. Future me
  will cut corners on schema migration the same way. Add a JSON Schema
  + a forward-compat parsing test.

## Code-quality cleanups

- [x] **C1. T3 SQL string concatenation.** Layers filter is built via
  `format!()` against a hard-coded allowlist (safe today). Convert to
  parameterized queries or push the allowlist check into a typed enum
  before format-time so the pattern doesn't get copy-pasted unsafely
  later.

- [x] **C2. W3.5 `app_error_logged!` macro is unused.** Shipped but no
  call site uses it -- the auto-log via IntoResponse covers the basic
  case. Either remove the macro (dead code) or use it in 5-10 of the
  highest-context error sites in `capsem-service/src/main.rs` so it
  earns its keep.

- [x] **C3. capsem-app `service.start` line absent.** Project invariant
  blocks adding a capsem-core dep. But the support-bundle parser keys
  on the `protocol_version`+`schema_hash` fields of that line for
  cross-version-mix detection. capsem-app is the only host binary that
  doesn't emit it, so a bundle with a capsem-app log won't surface
  version skew on that side. Either inline the same emission in
  capsem-app's bootstrap, or document the carve-out.
