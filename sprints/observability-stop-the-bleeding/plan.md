# Sprint plan: observability — stop the bleeding (full meta-sprint)

## Context

Capsem hits a recurring failure mode: silent IPC drops, swallowed errors, missing timing data, and protocol skew across binaries built at different commits all conspire to make real bugs un-diagnosable from logs alone. The original audit (`sprints/observability-stop-the-bleeding/ISSUE.md`) catalogued ~85 P0/P1 sites across six patterns. This sprint expands that scope to also (a) make `capsem-mcp` an effective triage surface so an agent or developer can zoom in on any failure in one or two tool calls, (b) ship a `capsem support-bundle` command so users can hand us a single tar.gz when reporting bugs, and (c) lay W3C-traceparent groundwork for a later OpenTelemetry pass — without committing to OTel as a runtime dep yet.

The intended outcome: the next bug is findable in one read of the logs (or one `capsem_panics` call), and the bug after that, and the one after that. The instrumentation is what's load-bearing; the bug fix is the cheap part.

User decisions locked in: full meta-sprint scope, OTel layer skipped this sprint (W3C traceparent propagated as plain strings; OTel layer is additive in a future sprint), accept the Frame<T> wire break (the handshake error IS the cross-version detection mechanism), normalize all 9 binaries to JSON file logs under `~/.capsem/run/`.

## Sub-sprint sequence

Eleven sub-sprints, sequenced for incremental commitability. Each row is one functional milestone / one PR. Wire stages are W1–W6; dev-tooling stages are T1–T5.

| # | Sub-sprint | Depends on | Why this position |
|---|------------|------------|-------------------|
| 1 | **W1**: `try_send!` macro + IPC/vsock send-site codemod | — | Highest-blast-radius single change; closes parent acceptance criterion #1 |
| 2 | **W2**: `init_telemetry()` consolidation + JSON normalization for all 9 binaries + gateway/tray file logs | — | Unblocks every later sub-sprint that reads logs; T1 support-bundle parser depends on consistent JSON |
| 3 | **W3 + W3.5**: Frame<T> + Hello handshake + `app_error_logged!` macro | W1, W2 | Closes parent #2/#3/#4/#5 atomically; bumps schema_hash so post-W3 builds reject pre-W3 peers |
| 4 | **W4**: env-var W3C traceparent propagation (no OTel) + `#[instrument]` on suspend/resume/MCP/fsync hot paths | W2 | Closes parent #6; parent-id grep now works across processes |
| 5 | **W5**: in-band traceparent on `Frame::Msg` + vsock envelope + JSON-RPC `_meta` | W3, W4 | Span-tree across processes; needs handshake bump and traceparent values to put on the wire |
| 6 | **W6**: `trace_id` columns on remaining 7 session.db tables + populate from ambient context | W4 | Independent of W5; unblocks T2/T3 timeline tooling |
| 7 | **T1**: `capsem support-bundle` skeleton + redactor + manifest v1 | W2 | First user-facing deliverable; lands "logs in an instant" goal |
| 8 | **T2**: `capsem_triage` + `capsem_panics` MCP tools | W2, W3.5 | Reads consolidated JSON logs; `app_error_logged` events make ranking deterministic |
| 9 | **T3**: `capsem_timeline` + `capsem_host_logs` MCP tools | W6 | Cross-table trace_id joins now possible |
| 10 | **T4**: `capsem-doctor --bundle` + support-bundle integration of doctor output | T1 | Adds in-VM artifact export; support bundle becomes complete |
| 11 | **T5**: CI artifact uploads on failure + `just test-artifacts` recipe + frontend `__capsemDebug` console hook | — | Independent; ship after the rest is stable |

Plus parent ISSUE tasks #5 (`_ => {}` audit, top 5 highest-blast-radius arms) and #6 (`unwrap_or_default()` codemod for high-blast-radius sites) absorbed into W3 (the protocol-touching ones) and W4 (the parsing ones).

Frontend log-forwarding hardening (parent #7) → punt to `sprints/frontend-rebuild`. T5 lands the minimal `__capsemDebug` console hook only.

---

## W1: `try_send!` macro and codemod

**New file:** `crates/capsem-core/src/macros.rs` — `try_send!` macro with two arms (async for `tokio::sync::mpsc::Sender`, sync for `broadcast`/`oneshot`/`std::sync::mpsc`). Logs `target: "ipc"` with `channel`, `error` fields on send failure. Exposed via `#[macro_export]`. Add `pub mod macros;` to `crates/capsem-core/src/lib.rs`.

**Codemod sites (54 total):**
- `crates/capsem-process/src/vsock.rs` — ~21 sites (largest cluster). Channel names: `terminal_rekey`, `control_rekey`, `term_in`, `hub`, `ipc_state_change`, `ctrl_lifecycle`, `job_result_*`.
- `crates/capsem-process/src/ipc.rs` — ~14 sites. Channels: `ipc_out`, `ctrl_forward`.
- `crates/capsem-process/src/main.rs` — ~3 sites.
- `crates/capsem-service/src/main.rs` — ~10 sites. Channels: `shutdown_ipc`, `suspend_recv`.
- `crates/capsem/src/main.rs` — ~6 sites.

**Critical case: `vsock.rs:579-585` lifecycle thread tail** — converts `let _ = itx.send(ProcessToService::ShutdownRequested { id })` and `let _ = ctx.blocking_send(ServiceToProcess::Shutdown)` to `try_send!`. These are the exact lines that wedged investigations earlier.

**Exemptions:** legitimate cleanup-path `let _ = X.send(...)` (Drop impls, oneshot where receiver-cancellation is the design) keep `let _ =` plus a `// channel-closed-ok: <reason>` trailing comment. Acceptance grep excludes that comment.

**Acceptance:** `rg 'let _ = .*\.send\(' crates/capsem-process crates/capsem-service crates/capsem/src | grep -v 'channel-closed-ok'` returns zero hits.

---

## W2: `init_telemetry()` consolidation + JSON normalization

**New file:** `crates/capsem-core/src/telemetry.rs` — a single `init(cfg: TelemetryConfig) -> Result<TelemetryGuard>` function every binary calls in `main()`. `TelemetryConfig { service: &'static str, sink: LogSink, default_filter: &'static str }`. `LogSink` = `Stderr | File { path } | FileAndPretty { path }`. The guard owns a `tracing_appender::non_blocking::WorkerGuard` and is held for the lifetime of `main()`.

**Skip the OTel layer this sprint** — no `opentelemetry`, `opentelemetry-otlp`, or `tracing-opentelemetry` deps. The function reads `TRACEPARENT` env var (W4) and stashes it in a process-global `OnceLock<String>` so the in-band traceparent helpers (W5) can pull it. Adding the OTLP exporter later is purely a layer addition; the API stays stable.

**Conversion table** (replace per-binary boilerplate with `let _g = capsem_core::telemetry::init(cfg)?;`):

| Binary | File:line | New sink | Format change |
|--------|-----------|----------|---------------|
| capsem-service | `main.rs:2890-2902` | `File { ~/.capsem/run/service.log }` | already JSON |
| capsem-process | `main.rs:111-114` | `Stderr` | already JSON |
| capsem-mcp | `main.rs:657-668` | `File { ~/.capsem/run/mcp.log }` | already JSON |
| capsem-app | `main.rs:230-241` | `FileAndPretty { ~/.capsem/logs/<ts>.jsonl }` | already JSON |
| capsem-mcp-aggregator | `main.rs:47-54` | `Stderr` | already JSON |
| **capsem-gateway** | `main.rs:66-75` | `File { ~/.capsem/run/gateway.log }` | **compact → JSON** |
| **capsem-tray** | `main.rs:47-52` | `File { ~/.capsem/run/tray.log }` | **compact → JSON** |
| **capsem-mcp-builtin** | `main.rs:368-372` | `Stderr` | **compact → JSON** |
| capsem (CLI) | n/a | n/a | unchanged (CLI has its own user-facing output) |

Guest agent (`capsem-agent`) keeps its `blog_line` text writer for now (musl-static binary size matters). W4 adds `trace_id=<hex>` to every line as a 5-line change to `blog_line()`.

**Acceptance:** every long-lived binary writes JSON under `~/.capsem/run/`; `service.start` log line at init includes `protocol_version=1, schema_hash=<hex>` for cross-version-mix detection; `tail -f ~/.capsem/run/gateway.log` shows valid JSON.

---

## W3: Frame<T> + Hello handshake (parent #2, #4)

**New file:** `crates/capsem-proto/src/handshake.rs` — `Hello { version: u16, schema_hash: u64, peer: String, traceparent: String }`, `HandshakeError`, and `pub const PROTOCOL_VERSION: u16 = 1`. `pub const SCHEMA_HASH: u64 = include!(concat!(env!("OUT_DIR"), "/schema_hash.txt"))` — emitted by build script.

**New file:** `crates/capsem-proto/build.rs` — FNV-1a 64-bit hash over the source bytes of `src/lib.rs` and `src/ipc.rs`. Emits `cargo:rerun-if-changed` for both. Hand-rolled (7-line FNV); no new crate dep. Comment edits trip the hash; that's a fast-and-loud cost we accept.

**Bincode wrapper** — every typed IPC channel becomes:
```rust
#[derive(Serialize, Deserialize)]
pub enum Frame<T> { Hello(Hello), Msg { payload: T, trace: Option<String> } }
```

`trace: Option<String>` is the per-message override slot for W5. Default `None` → use connection-scoped traceparent from the Hello.

**Wire break is intentional and accepted:** v0 binaries that send a raw enum first will fail decode on a v1 reader. The handshake's structured error log (`target="ipc"`, `ours_hash`, `peer_hash`, `peer`) is precisely the cross-version-mix detection mechanism. The original ISSUE.md complains about a 30-second silent timeout; post-W3 it's a 1-second loud `error!`.

**`negotiate()` helper** — new module `crates/capsem-core/src/ipc_handshake.rs` with `negotiate_initiator` and `negotiate_responder` for bincode channels. 5-second receive timeout for the v0-peer-never-sends-Hello case. On mismatch: `tracing::error!(target: "ipc", site, ours = ..., peer = ..., peer_hash = ..., our_hash = ..., "protocol handshake failed; one side is older than the other")`.

**Six bincode call sites that need negotiate (verified by Plan agent):**
1. `crates/capsem-process/src/ipc.rs:24` (server-side responder)
2. `crates/capsem-service/src/main.rs:1582` (client-side initiator, `send_ipc_command`)
3. `crates/capsem-service/src/main.rs:2356` (graceful Shutdown)
4. `crates/capsem-service/src/main.rs:2420` (`handle_suspend`)
5. `crates/capsem/src/main.rs:497` (CLI shell IPC)
6. `crates/capsem/src/main.rs:1504` (orphan-cleanup graceful Shutdown)

**Two vsock control-bridge sites** (uses RMP not bincode; reuse existing `[u32 BE len][rmp]` framing via new `encode_hello`/`decode_hello` in `crates/capsem-proto/src/lib.rs`):
7. `crates/capsem-process/src/vsock.rs:656` (`perform_handshake`, host-side responder)
8. `crates/capsem-agent/src/main.rs:154` (just before first `Ready`, guest-side initiator)

**MCP NDJSON path** — add optional `_meta: Option<JsonRpcMeta>` to `JsonRpcRequest` / `JsonRpcResponse` in `crates/capsem-core/src/mcp/types.rs`. `JsonRpcMeta { traceparent: String, tracestate: Option<String>, schema_hash: Option<u64> }`. Best-effort version stamp only — third-party MCP servers won't speak our handshake; we log `target="mcp"` warn on mismatch but don't reject.

**Acceptance test:** `tests/capsem-service/test_protocol_handshake.py` builds a synthetic v0 client (raw `bincode::serialize(&ServiceToProcess::Ping)` with no Frame wrapper), sends to service UDS, asserts connection closes within 1s and service.log has the structured handshake error with both schema hashes.

---

## W3.5: `app_error_logged!` macro (parent #3)

**Co-located with `AppError`:** `crates/capsem-service/src/errors.rs`. Two macro arms (with-fields and bare-fmt). Expands to `tracing::{lvl}!(target: "service", status = ?$status, $($fields)+, "{}", __msg)` followed by `AppError($status, __msg)`.

**Codemod scope:** 103 `AppError(StatusCode, ...)` sites in `capsem-service/src/main.rs`. Triage by error class:
- ~40 `.map_err(|e| AppError(...))` chains → `app_error_logged!(error, ...)`.
- ~30 `Err(AppError(...))` direct returns → same.
- ~20 `BAD_REQUEST` / `NOT_FOUND` user-error sites → `app_error_logged!(warn, ...)` (4xx is the user's problem; warn not error).

**Acceptance grep:** `rg 'AppError\(StatusCode' crates/capsem-service/src/main.rs` shows every site is either the expansion of `app_error_logged!` or has a preceding `tracing::{error,warn}!` on the same logical statement.

---

## W4: env-var W3C traceparent propagation + hot-path spans (parent #6)

**Helper in `capsem-core::telemetry`:**
```rust
pub fn child_trace_env(vm_id: &str) -> Vec<(String, String)>
```
Returns the four-pair tuple `[CAPSEM_VM_ID, CAPSEM_TRACE_ID, TRACEPARENT, TRACESTATE]`. `TRACEPARENT` synthesized from the current span's `trace_id` field (or generated fresh if none) as a deterministic `00-<32hex>-<16hex>-01` W3C string. No OTel layer — these are just structured fields and env vars.

**Spawn sites that need updating:**
| File:line | Spawns | Currently passes |
|-----------|--------|------------------|
| `capsem-service/src/main.rs:418`, `:636` | capsem-process | only `CAPSEM_VM_ID` → add CAPSEM_TRACE_ID, TRACEPARENT, TRACESTATE |
| `capsem-process/src/main.rs:670` | capsem-mcp-aggregator | already does CAPSEM_TRACE_ID; add traceparent/tracestate |
| Service spawn paths for capsem-gateway, capsem-tray | none → add all four |

**Receive side:** `init_telemetry` reads `TRACEPARENT` and stashes the trace_id portion in a process-global `OnceLock<String>`. The root span (e.g. `capsem-process/main.rs:121`) reads from that and includes it as a structured field on every log line.

**`#[instrument(skip_all, fields(...))]` and explicit timing spans on:**
- `with_quiescence`, `save_state`, `restore_state`, `pause` in `capsem-core/src/hypervisor/apple_vz/`
- `attach_disk`, `attach_virtiofs_share` in same area
- `send_ipc_command`, `wait_for_vm_ready` in `capsem-service/src/main.rs`
- The rootfs.img fsync block in `capsem-process/src/vsock.rs::Suspend` — wrap with `start = Instant::now()` → `info!(target = "fs", op = "fsync", path = "rootfs.img", duration_ms = ...)`
- MCP tool dispatch in `capsem-core/src/mcp/gateway.rs`

**Guest-side trace plumbing:** `BootConfig` (the first message host→guest) gains a `traceparent: String` field. `capsem-agent`'s `blog_line` reads it on boot and includes `trace_id=<lower 16 hex>` in every line. 5-line change.

**`_ => {}` audit (parent #5):** convert the top 5 highest-blast-radius arms to `unhandled => warn!(?unhandled, target = "ipc", "unknown variant; this binary may be older than its peer")`:
- `capsem-process/vsock.rs:570` (lifecycle port)
- `capsem-process/vsock.rs:596-645` (handle_guest_msg)
- `capsem-mcp-aggregator/main.rs:106-108`
- `capsem-process/ipc.rs:306-308`
- vsock control bridge main match

**Acceptance:** suspend produces JSON spans `quiescence`, `apple_vz_pause`, `apple_vz_save_state`, `host_fsync_rootfs`, each with `duration_ms`. `tests/capsem-service/test_svc_loop_device_after_resume.py` either passes or fails with a useful message pointing at the right host-side step.

---

## W5: in-band traceparent on Frame::Msg + vsock envelope

**Bincode side:** `Frame::Msg { payload, trace: Option<String> }` already in W3. Send-sites that originate under a span with a different trace than the connection root (broadcast events, async StateChanged, etc.) pass `Some(current_traceparent())`. Codemod from W1 made these sites already touched once; this is a second pass that's mostly mechanical.

**Vsock envelope upgrade** — wrap `HostToGuest` / `GuestToHost` in:
```rust
#[derive(Serialize, Deserialize)]
struct GuestEnvelope { msg: HostToGuest, #[serde(default, skip_serializing_if = "String::is_empty")] trace: String }
```
in `crates/capsem-proto/src/lib.rs`. `encode_host_msg` / `encode_guest_msg` (lines 270-298) gain a `traceparent: &str` arg; existing callers that don't have one pass `""`. Schema_hash bumps because the wire shape changes.

**MCP `_meta.traceparent`** — already added in W3 §4.5. Each `mcp_calls` row gets the traceparent from the request's `_meta` field if present.

**Acceptance:** a single `just run "echo hi"` produces log lines with the SAME `trace_id` field across capsem-service, capsem-process, capsem-mcp-aggregator, AND capsem-agent's `/var/log/capsem-boot.log`.

---

## W6: trace_id on every session.db table

**Migrations** — add to `idempotent_migrations` block at `crates/capsem-logger/src/schema.rs:209-310`:
```rust
let _ = conn.execute("ALTER TABLE mcp_calls       ADD COLUMN trace_id TEXT", []);
let _ = conn.execute("ALTER TABLE net_events      ADD COLUMN trace_id TEXT", []);
let _ = conn.execute("ALTER TABLE fs_events       ADD COLUMN trace_id TEXT", []);
let _ = conn.execute("ALTER TABLE snapshot_events ADD COLUMN trace_id TEXT", []);
let _ = conn.execute("ALTER TABLE tool_calls      ADD COLUMN trace_id TEXT", []);
let _ = conn.execute("ALTER TABLE tool_responses  ADD COLUMN trace_id TEXT", []);
let _ = conn.execute("ALTER TABLE audit_events    ADD COLUMN trace_id TEXT", []);
// + CREATE INDEX IF NOT EXISTS for each
```

**Event-struct additions** in `crates/capsem-logger/src/events.rs`:
- `McpCall` (line 159) — populated in `gateway.rs` from ambient span context
- `FsEvent` — populated in `fs_monitor.rs` from ambient span
- `NetEvent` — already has `TraceState`; thread it through to the writer
- `SnapshotEvent` — capture parent span at scheduler-creation time, `.in_current_span()` the spawn
- `AuditEvent` — guest-originated; populated only after W4's BootConfig.traceparent plumbing

**Helper** `capsem_core::telemetry::ambient_capsem_trace_id() -> Option<String>` — walks tracing::Span for `trace_id` field; falls back to env `CAPSEM_TRACE_ID`. Every event constructor calls this when it doesn't have an explicit trace_id to pass.

**Acceptance:** for a single tool-call session, `SELECT DISTINCT trace_id FROM net_events UNION SELECT DISTINCT trace_id FROM mcp_calls UNION SELECT DISTINCT trace_id FROM exec_events` returns one row.

---

## T1: `capsem support-bundle`

**New CLI subcommand:** `Misc::SupportBundle { output: Option<PathBuf>, sessions: usize, include_rootfs: bool, no_redact: bool }`.

**Defaults:**
- `--output`: `~/.capsem/support/capsem-support-<ts>-<host>.tar.gz`
- `--sessions 3`: include last N session dirs by mtime, max 10
- `--include-rootfs`: off (footgun: 2GB+)
- `--no-redact`: off

**Bundle layout:**
```
capsem-support-20260502-184303-elie/
  manifest.json                          # entry point; written last
  host/{service,mcp,gateway,tray}.log    # last 5MB each, tail-trimmed at line boundary
  host/app/<latest 3>.jsonl
  host/run-snapshot/{service.pid,gateway.pid,gateway.port}   # gateway.token REDACTED
  sessions/<id>/{session.db,serial.log,process.log,metadata.json,doctor.tar?}
  assets/manifest.json
  config/{user.toml,corp.toml,corp-source.json}              # secrets redacted
  system/{version.json,os.txt,proxy.json,dmesg.log,mitm-ca-fingerprint.txt}
  doctor/output.txt                                          # ~/.capsem/run/doctor-latest.log
```

**Redactor module** at `crates/capsem/src/support/redact.rs`:
1. TOML/JSON keys matching `(?i)(token|secret|api[_-]?key|password|authorization|gateway[_-]?token|github[_-]?token)$` → `"<redacted>"`.
2. `Authorization: Bearer \S+` → `Bearer <redacted>` in log lines.
3. API key prefixes `sk-…`, `AIza…`, `xox[baprs]-…` → `<redacted-key>`.
4. `/Users/<x>/` and `/home/<x>/` → `~/`.
5. MITM CA fingerprint goes plaintext (it's a fingerprint, not the cert).

**Manifest schema v1** — `schema_version: 1, generated_at, generator { binary, version, build_hash, platform }, host { hostname, os, shell }, capsem_home, redacted, sections [{ path, kind, lines?, bytes?, missing?, reason?, truncated_to_last_bytes?, session_id?, tables? }], warnings, next_steps`. `schema_version` is the only forward-compat lever.

**No `--upload` flag this sprint.** Out of scope.

**Tests:** `tests/capsem-install/test_support_bundle.py` — happy path, redaction (grep -E `Bearer [A-Za-z0-9]+|sk-[A-Za-z0-9_-]{20,}` returns zero), missing-file handling, manifest schema validation.

---

## T2: `capsem_triage` + `capsem_panics` MCP tools

**`capsem_triage`** — opinionated host+session error summary. Params: `id?: String, since?: String, limit?: usize`.
- Tail-reads (last ~1MB) and JSON-parses: `service.log`, `mcp.log`, `gateway.log`, `tray.log`, `~/.capsem/logs/<latest>.jsonl`. Filter to `level >= WARN` within `since`.
- Detects panics with regex `\bpanicked at\b|^thread '[^']+' panicked` and groups consecutive backtrace frames.
- Surfaces W3.5/W1 markers: `target=ipc` warns (the `try_send!` macro), `target=fs op=fsync` slow ops (>500ms), `target=service` `app_error_logged` events.
- If `id` given: queries session.db for net `denied`/5xx, mcp `denied`/error, exec failures.
- Returns `{ since, host { panics, errors, slow_ops }, session { exec_failures, denied_net, mcp_errors }, rank: [...] }`. The `rank` array is a hard-coded heuristic: panics > unhandled enum warns > slow_op+error correlation.

**`capsem_panics`** — focused panic + backtrace extractor. Params: `since?: String, limit?: usize`.
- Scans all host logs + last 10 sessions' `process.log`.
- Two-pass parse: structured tracing events with `panic = ...` field, plus plain-text fallback regex.
- Returns `[{ ts, binary, thread, location, message, frames: [...] }]`. Up to 16 frames; redacts `/Users/<u>/` → `~`.

**Implementation:**
- New service HTTP endpoints `/triage`, `/panics` (snake_case) gated by existing UDS auth.
- New module `crates/capsem-service/src/triage.rs` with panic regex + JSON-line decoder + lookback parser. Unit tests against fixtures in `crates/capsem-service/tests/fixtures/logs/`.
- Reusable `parse_since("30m" | "2h" | RFC3339) -> SystemTime` lives in `capsem-core::time`.
- Two new tools wired in `capsem-mcp/src/main.rs:462+` (the `#[tool_router]` impl).

**Acceptance:** `capsem_triage --id <x>` against a session with one panic and one slow fsync ranks the panic first. `capsem_panics` finds an injected `panic!()` in capsem-gateway within 1s.

---

## T3: `capsem_timeline` + `capsem_host_logs` MCP tools

**`capsem_timeline`** — trace-id-joined event stream. Params: `id: String, trace_id?: String, since?: String, limit?: usize, layers?: Vec<String>`. UNION query over `exec_events`, `mcp_calls`, `net_events`, `fs_events`, `model_calls` ordered by timestamp. When `trace_id` is given, filter every layer by it (W6 makes this work everywhere). Joins `tool_calls.mcp_call_id` to attach MCP context.

**`capsem_host_logs`** — read any host-side log by symbolic name. Params: `name: "service"|"mcp"|"gateway"|"app"|"tray", grep?: String, tail?: usize, max_bytes?: usize`. Hard-coded allowlist (no path traversal). Resolves via `capsem_core::paths`.

**Implementation:**
- New service HTTP endpoints `/timeline/{id}`, `/host-logs/{name}` (snake_case).
- The "app" name resolves to the newest file in `~/.capsem/logs/`.
- Tools added to `capsem-mcp/src/main.rs:462+`.

**Acceptance:** `capsem_timeline --id <x> --trace-id <t>` returns at least one row from each of `model`, `mcp`, `exec` for a known tool-call chain. `capsem_host_logs --name service --tail 50 --grep ERROR` returns the matching tail.

---

## T4: `capsem-doctor --bundle` + support-bundle integration

**Rewrite** `guest/artifacts/capsem-doctor` (~70 lines bash) to accept `--bundle [PATH]`:
- Defaults to `/shared/doctor-bundle.tar` (verify the actual virtiofs mount path against `crates/capsem-process/src/main.rs` virtiofs setup before shipping; fallback `/tmp/doctor-out.tar` then `cp /tmp/... /shared/`).
- Tar contents: `pytest-output.txt`, `pytest-junit.xml` (from `pytest --junitxml`), `session.db`, `var-log.tar` (`tar c -C /var/log .`), `dmesg.log`, `init.log` (`/tmp/capsem-init.log`), `proc-mounts.txt`, `proc-cmdline.txt`.
- Pytest filters (`-k`) still apply with `--bundle`.

**Host-side wrapper** in `crates/capsem/src/main.rs:1447-1610` (`Doctor`) — add `bundle: bool` flag. When true: append `--bundle /shared/...` to the typed command, after pytest exits read the tar via virtiofs, copy to `~/.capsem/run/doctor-latest.tar`.

**T1 integration** — support-bundle loop, when `~/.capsem/run/doctor-latest.tar` exists, embed it as `doctor/bundle.tar`.

**Acceptance:** `capsem doctor --bundle` from the host produces `~/.capsem/run/doctor-latest.tar`. `capsem support-bundle` after that includes both `doctor/output.txt` and `doctor/bundle.tar`.

---

## T5: CI artifact uploads + just recipe + frontend `__capsemDebug`

**`.github/workflows/ci.yaml`** — append on both `test:` (macOS) and `test-linux:` jobs:
```yaml
- name: Upload integration test artifacts on failure
  if: failure()
  uses: actions/upload-artifact@v4
  with:
    name: test-artifacts-${{ matrix.os || runner.os }}-${{ github.run_attempt }}
    path: |
      test-artifacts/
      frontend/test-artifacts/
    retention-days: 7
    if-no-files-found: ignore
```

**`justfile`** — `just test-artifacts` recipe: lists the latest preserved failure dir, prints the file list with sizes, hints at next commands.

**Frontend `__capsemDebug`** — `frontend/src/lib/tauri-log.ts` exposes `window.__capsemDebug` when `?debug=1` in URL. Methods: `dumpLogs()` (invokes new Tauri `dump_frontend_logs` command, returns latest jsonl path), `versions()` (`{ build_ts, app_version, api_socket }`), `lastWsEvents` (ring buffer of last 5 WS events; wrap the existing `frontend/src/lib/api.ts:391` onmessage). No UI panel — devs use the browser console. Visual panel punted to `sprints/frontend-rebuild`.

**Acceptance:** a failing `tests/capsem-service/test_*.py` run on a PR uploads `test-artifacts-macos-1`. `just test-artifacts` after a failure prints the path. `?debug=1` exposes `window.__capsemDebug`.

---

## Skill updates after each sub-sprint

- After **W1**: `skills/dev-rust-patterns/SKILL.md` — add the `try_send!` macro to the IPC patterns section.
- After **W2**: `skills/dev-installation/SKILL.md` — document `~/.capsem/run/{gateway,tray}.log` additions.
- After **W3**: `skills/dev-rust-patterns/SKILL.md` — handshake convention; `skills/dev-debugging/SKILL.md` — read schema_hash mismatch errors first.
- After **W4**+**W5**+**W6**: `skills/dev-session-debug/SKILL.md` — every table now has `trace_id`; trace_id propagates host→process→aggregator→guest.
- After **T2**+**T3**: `skills/dev-mcp/SKILL.md` — new tool table rows; `skills/dev-debugging/SKILL.md` — new triage workflow `capsem_panics` → `capsem_triage` → `capsem_timeline`.
- After **T1**+**T4**: `skills/dev-installation/SKILL.md` — `capsem support-bundle` and `capsem doctor --bundle`.
- After **T5**: `skills/dev-testing/SKILL.md` — CI artifact retrieval + `just test-artifacts`.

## CHANGELOG entries

One entry per sub-sprint commit under `## [Unreleased]`. From the user's perspective: e.g. "Added: `capsem support-bundle` collects logs and config into a single redacted tar.gz for bug reports." "Changed: protocol skew between service and capsem-process now fails fast at handshake instead of timing out." "Fixed: silent IPC drops in suspend/resume path now log at warn with channel name and error."

## Critical files to be modified

**New files:**
- `crates/capsem-core/src/macros.rs` (W1 `try_send!`)
- `crates/capsem-core/src/telemetry.rs` (W2 `init_telemetry`, W4 helpers, W6 ambient_trace_id)
- `crates/capsem-core/src/ipc_handshake.rs` (W3 negotiate)
- `crates/capsem-proto/src/handshake.rs` (W3 Hello, Frame, errors)
- `crates/capsem-proto/build.rs` (W3 schema_hash FNV)
- `crates/capsem-service/src/triage.rs` (T2 panic regex, since-parser, lookback scanner)
- `crates/capsem-service/tests/fixtures/logs/*.json` (T2 panic fixtures)
- `crates/capsem/src/support_bundle.rs` (T1 bundler)
- `crates/capsem/src/support/redact.rs` (T1 redactor)
- `tests/capsem-service/test_protocol_handshake.py` (W3 acceptance)
- `tests/capsem-install/test_support_bundle.py` (T1 acceptance)

**Modified, high-impact:**
- `crates/capsem-proto/src/lib.rs` (W3+W5 vsock envelope, encode/decode signature change)
- `crates/capsem-proto/src/ipc.rs` (W3 Frame<T> wrapper)
- `crates/capsem-proto/src/host_guest.rs` (W4 BootConfig.traceparent)
- `crates/capsem-process/src/vsock.rs` (W1 codemod, W3 perform_handshake, W4 spans, W5 envelope)
- `crates/capsem-process/src/ipc.rs` (W1 codemod, W3 Frame, W5 trace)
- `crates/capsem-process/src/main.rs` (W2 init_telemetry, W4 spawn env)
- `crates/capsem-service/src/main.rs` (W1 codemod, W2 init_telemetry, W3 negotiate at 3 sites, W3.5 codemod, W4 spawn env, T2/T3 endpoints)
- `crates/capsem-service/src/errors.rs` (W3.5 `app_error_logged!`)
- `crates/capsem-mcp/src/main.rs` (W2 init_telemetry, T2/T3 four new tools)
- `crates/capsem-mcp-aggregator/src/main.rs` (W1 unhandled arm, W2 init_telemetry, W4 spawn env)
- `crates/capsem-mcp-builtin/src/main.rs` (W2 init_telemetry, JSON normalization)
- `crates/capsem-gateway/src/main.rs` (W2 init_telemetry, file sink)
- `crates/capsem-tray/src/main.rs` (W2 init_telemetry, file sink)
- `crates/capsem-app/src/main.rs` (W2 init_telemetry, T5 dump_frontend_logs Tauri cmd)
- `crates/capsem-agent/src/main.rs` (W3 vsock handshake, W4 trace_id in blog_line)
- `crates/capsem-logger/src/schema.rs` (W6 migrations)
- `crates/capsem-logger/src/events.rs` (W6 trace_id on 7 structs)
- `crates/capsem-logger/src/writer.rs` (W6 plumb trace_id through INSERTs)
- `crates/capsem-core/src/mcp/types.rs` (W3 JsonRpcMeta)
- `crates/capsem-core/src/mcp/gateway.rs` (W4 instrument, W6 trace_id on McpCall)
- `crates/capsem-core/src/hypervisor/apple_vz/{boot,machine,suspend}.rs` (W4 instrument)
- `crates/capsem/src/main.rs` (W1 codemod, W3 negotiate at 2 sites, T1 SupportBundle, T4 Doctor `--bundle`)
- `guest/artifacts/capsem-doctor` (T4 `--bundle` rewrite)
- `frontend/src/lib/tauri-log.ts` (T5 `__capsemDebug`)
- `frontend/src/lib/api.ts:391` (T5 ws ring buffer)
- `.github/workflows/ci.yaml:212` (T5 upload-artifact step)
- `justfile` (T5 `test-artifacts` recipe)

## Verification

Run after every sub-sprint commit, in order:
1. `just test` — full unit + integration + cross-compile + frontend gates.
2. `just smoke` — doctor + integration tests in a real VM.
3. `just inspect-session` after a session — verify telemetry pipeline (especially after W6 — `trace_id` non-NULL on every event class).

Sub-sprint-specific checks:
- **W1:** `rg 'let _ = .*\.send\(' crates/capsem-process crates/capsem-service crates/capsem/src | grep -v 'channel-closed-ok'` → zero hits.
- **W2:** `tail -f ~/.capsem/run/{service,mcp,gateway,tray}.log` → all valid JSON; first line of each contains `protocol_version=1, schema_hash=<hex>`.
- **W3:** run `tests/capsem-service/test_protocol_handshake.py` — synthetic v0 client gets a 1-second loud handshake error, not a 30-second timeout.
- **W3.5:** `rg 'AppError\(StatusCode' crates/capsem-service/src/main.rs` → every site is preceded by tracing or uses `app_error_logged!`.
- **W4:** run `just run "echo hi"` then grep `process.log` for `op=fsync` → see `duration_ms=...`. Suspend produces `quiescence`, `apple_vz_pause`, `apple_vz_save_state`, `host_fsync_rootfs` spans.
- **W5:** `just run "echo hi"` → grep all four logs (service, process, mcp-aggregator, capsem-boot.log inside guest) for the same `trace_id` value.
- **W6:** in any session, `SELECT DISTINCT trace_id FROM net_events UNION SELECT DISTINCT trace_id FROM mcp_calls UNION SELECT DISTINCT trace_id FROM exec_events` returns 1 row.
- **T1:** `capsem support-bundle` exits 0; extracted bundle's manifest validates; `grep -rE 'Bearer [A-Za-z0-9]+|sk-[A-Za-z0-9_-]{20,}' <extracted>/` returns nothing.
- **T2:** inject a `panic!()` in capsem-gateway, restart, run `capsem_panics` from MCP — finds it within 1s with location and frames.
- **T3:** `capsem_timeline --id <x> --trace-id <t>` returns ≥1 row from each layer for a known tool-call.
- **T4:** `capsem doctor --bundle` produces `~/.capsem/run/doctor-latest.tar`; `capsem support-bundle` after that includes `doctor/bundle.tar`.
- **T5:** force a CI failure on a draft PR — confirm `test-artifacts-*` artifact uploaded. `just test-artifacts` after a local failure prints the path.

End-state demonstration: replay the today-2026-05-02 silent-IPC bug with the new instrumentation. Expect handshake error log within 1 second naming both schema hashes and binaries; no 30-second timeout. That single end-to-end test is the sprint's load-bearing acceptance criterion.
