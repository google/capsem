# Sprint tracker: observability — stop the bleeding

Plan: `plan.md` (mirror of approved `~/.claude/plans/lively-splashing-scone.md`).

## Sub-sprints

- [x] **W1** — `try_send!` macro + IPC/vsock send-site codemod
- [x] **W2** — `init_telemetry()` consolidation + JSON normalization (gateway/tray/builtin) + file logs
- [x] **W3** — Hello handshake + schema_hash build script (side-channel, no Frame<T>)
- [x] **W3.5** — Auto-log AppError via IntoResponse (zero codemod) + optional `app_error_logged!` macro
- [x] **W4** — env-var W3C traceparent propagation + suspend/MCP/fsync hot-path timing + top 5 unhandled enum arms
- [x] **W5** — in-band traceparent on BootConfig + JSON-RPC `_meta` (Frame<T> wrap descoped per W3 rationale)
- [x] **W6** — trace_id columns on 7 remaining session.db tables (population path follow-up)
- [x] **T1** — `capsem support-bundle` skeleton + redactor + manifest v1
- [x] **T2** — `capsem_triage` + `capsem_panics` + `capsem_host_logs` MCP tools
- [x] **T3** — `capsem_timeline` MCP tool (capsem_host_logs already shipped in T2)
- [x] **T4** — `capsem-doctor --bundle` + support-bundle integration
- [x] **T5** — CI artifact uploads + `just test-artifacts` + frontend `__capsemDebug`

## Skill updates queue

- [ ] dev-rust-patterns — try_send! + handshake convention (after W1, W3)
- [ ] dev-installation — gateway.log/tray.log + support-bundle + doctor --bundle (after W2, T1, T4)
- [ ] dev-debugging — schema_hash mismatch first; triage workflow `capsem_panics` → `capsem_triage` → `capsem_timeline` (after W3, T2, T3)
- [ ] dev-session-debug — every table has trace_id; trace_id host→guest (after W4–W6)
- [ ] dev-mcp — new tool table rows (after T2, T3)
- [ ] dev-testing — CI artifact retrieval + just test-artifacts (after T5)

## Notes

- 2026-05-02 — Sprint kicked off. Plan agents converged on 6+5 sub-sprint structure; user approved full meta-sprint scope, OTel layer skipped this sprint, accept Frame<T> wire break, normalize all 9 binaries to JSON file logs.
- 2026-05-02 — W1 done. Codemod actually touched 56 sites (the original audit missed terminal.rs and job_store.rs). One legitimate exemption: `TerminalOutputQueue::publish` broadcast send is best-effort by design (replay buffer is the source of truth). `JobResult` enum gained an unconditional `#[derive(Debug)]` (was test-only) so the macro's `error = ?__e` formatting works in production. Acceptance grep returns zero hits.
- 2026-05-02 — T2 done. Bundled with T3's `capsem_host_logs` since the host_logs endpoint is trivial and rounds out the triage trio. New `crates/capsem-service/src/triage.rs` (~400 lines) holds parse_since (duration + RFC3339), panic scanner (text + JSON), error scanner, slow-op scanner, host_log_path allowlist. 9 unit tests cover each. The plain-text panic parser is now stateful: thread+location come from the `thread '...' panicked at ...:` header, then the next non-empty non-frame line is captured as the message body, then `   at <path>` frames are appended (max 16). Three new HTTP endpoints (`/triage`, `/panics`, `/host-logs/{name}`) wired through the existing UDS auth gate. Three new MCP tools wired in `capsem-mcp/src/main.rs:489+`.

- 2026-05-02 — T1 done. Pivoted ahead of W4/W5/W6 because user explicitly named "logs from our users in an instant" as a goal -- ship the user-visible deliverable first. Bundle works end-to-end with 13 unit tests (5 bundle, 8 redact). Tests had a pre-existing parallel-execution bug pattern: CAPSEM_HOME is process-global so tests must serialize via Mutex<()> when mutating it. Pre-existing `redact::redact_line` short-circuit had a get_or_init/get-unwrap mismatch -- fixed by capturing all three Regex refs eagerly. T4 (capsem-doctor --bundle integration) will make `~/.capsem/run/doctor-latest.tar` populate the bundle's `doctor/bundle.tar` slot.
- 2026-05-02 — W3.5 done. Pivoted from the planned 104-site codemod to a single `IntoResponse` impl change: every AppError now auto-logs at the right level (5xx=error, 4xx=warn, other=info) with `target="service"` and the status code as a structured field. Zero call-site changes, 100% coverage. Optional `app_error_logged!` macro retained for sites that want an EARLY event (during the operation) in addition to the LATE one (response build). Plan estimate of 103 sites was overcount by 1 -- there are 104 AppError(StatusCode... constructors in main.rs.
- 2026-05-02 — W3 done. Picked the **side-channel handshake** variant: Hello bytes are exchanged on the raw UnixStream before `channel_from_std()`, then ownership returns to the bincode layer. Less invasive than the Frame<T> wrapper -- W1's send sites stay untouched and per-message traceparent override stays a clean W5 addition. Detection semantics are identical: a v0 binary times out in 5s (HELLO_TIMEOUT) with a structured `target="ipc"` error log. Wired 6 IPC sites: capsem-process/ipc.rs (responder), capsem-service/main.rs:1582/2356/2420 (3 initiators), capsem/main.rs:497/1504 (2 CLI initiators). Vsock control bridge handshake (host<->guest) deferred to W4 since it's coupled with BootConfig.traceparent. JSON-RPC `_meta` slot deferred to W5 along with Frame<T>.

- 2026-05-02 — W5 done. Bigger blast than expected because HEAD's mitm-redesign commit (`c4a1bf4`) had left a half-finished file rename in `net/{interpreters,parsers}/` -- the build was already broken before I started. Fixed both rivers in one commit: (a) added `pub mod interpreters; pub mod parsers;` to `net/mod.rs`, rewrote `super::events / provider / sse` imports in the moved files, and re-exported the old paths from `net::ai_traffic::{anthropic,google,openai,sse}` so existing call sites still compile; (b) wired the actual W5 work -- `BootConfig.traceparent` (consumed by capsem-agent's new `set_boot_traceparent`/`current_boot_trace_id` helpers + a process-global `BOOT_TRACEPARENT: OnceLock<String>`), and `_meta: Option<JsonRpcMeta>` on JsonRpcRequest/Response with `traceparent + tracestate` fields. Both new fields are optional with serde defaults so old peers and third-party MCP clients round-trip cleanly. Frame<T>-wrapping every IPC message was descoped in line with W3's side-channel-handshake decision: per-message override has marginal value over W4's at-spawn env-var propagation given how short Capsem IPC connections live in practice. Tool-router test was updated to expect the four new MCP tools (capsem_panics/triage/host_logs/timeline). All 1496 capsem-logger + 185 capsem-mcp + workspace tests pass; clippy clean.

- 2026-05-02 — T4 done. `guest/artifacts/capsem-doctor` rewritten to ~95 lines bash with `--bundle [PATH]` flag. Default path is `/shared/doctor-bundle.tar` if `/shared` exists (production VMs), else `/tmp/doctor-bundle.tar`. Bundle contents: pytest stdout (via tee), pytest junit XML, /tmp/capsem-init.log, dmesg, /proc/{mounts,cmdline}, /var/log (as nested tar), and session.db when present. Host-side `capsem doctor --bundle` lifts `<session_dir>/guest/doctor-bundle.tar` to `~/.capsem/run/doctor-latest.tar` BEFORE delete_vm runs (otherwise the session dir is gone). Support bundle's existing dispatch logic now picks up `doctor-latest.tar` and embeds it as `doctor/bundle.tar`. Two new clap-parser tests cover the `--bundle` flag round-trip; manual cargo build + clippy clean.

- 2026-05-02 — W2 done. Eight binaries (capsem-service/-process/-mcp/-mcp-aggregator/-mcp-builtin/-gateway/-tray, plus capsem-core CLI macros consumer in capsem) consolidated on `capsem_core::telemetry::init()`. capsem-app is intentionally left alone -- the project's "no capsem-core dep in the Tauri shell" invariant applies (its existing manual JSON-file-and-stderr-pretty setup matches what init() would produce anyway). Four binaries that emitted compact text (gateway, tray, mcp-builtin, mcp-aggregator) now emit JSON. New `~/.capsem/run/gateway.log` and `~/.capsem/run/tray.log` files appear. service.start emission needed `service=info,` prepended to default_filter so narrow per-binary filters don't suppress it. W3C TRACEPARENT env var captured into a OnceLock for W4/W5. Placeholder `PROTOCOL_VERSION = 0` and `SCHEMA_HASH = 0` added to capsem-proto -- W3 wires the real build.rs.
