# Sprint: observability — stop the bleeding

## TL;DR

Capsem has a project-wide observability deficit. The pattern surfaces
every time we hit a real bug: hours are wasted because something
silent (a dropped IPC frame, a swallowed parse error, an unmeasured
fsync) makes the failure unreproducible from logs alone.

This sprint installs the smallest set of changes that makes the
*next* bug findable in one read of `process.log` / `service.log` /
`dmesg`, instead of needing live instrumentation against a running
service.

Concrete trigger: 2026-05-02 session burned ~5 hours on what turned
out to be (a) a single missed `fsync(rootfs.img)` whose absence was
invisible because nothing logs fsync duration, and (b) an
`enum-variant-mismatch` between `capsem-service` (built before
commit `ccb4a4d`) and `capsem-process` (built after) where the IPC
read silently swallowed the serde decode error and just closed the
connection. The audit triggered by that session (see
`audit/findings.md` co-located with this ISSUE) found ~85 P0/P1
gaps following the same handful of patterns.

## Audit summary (the patterns)

A 9-domain audit (suspend/resume, IPC/vsock, network/MITM, MCP,
guest boot, telemetry/storage, CLI, gateway/HTTP/auth, frontend +
Tauri) produced findings that cluster into six recurring patterns.
Each pattern is the actual root cause; the per-file findings are
just symptoms.

1. **`let _ = X.send(...)` on IPC/vsock channels.** ~50 sites
   across `crates/capsem-process/src/{vsock.rs,ipc.rs}` and
   `crates/capsem-service/src/main.rs`. Every one of these can drop
   a critical message into the void with no log. Today's IPC
   protocol-skew bug went undetected for ~2 hours because the
   reader's `let _ = msg_tx.blocking_send(res)` discarded the
   decode-error result.

2. **`unwrap_or_default()` / `.ok().map().unwrap_or_default()` on
   parse + I/O.** ~12+ sites. `mitm_proxy.rs:765-794` (AI telemetry
   silently empties), `capsem-logger/writer.rs:311` (model_calls
   `usage_details` silently empty on bad data),
   `capsem-service/registry.rs:62-66` (persistent registry silently
   loads as empty on read failure -- losing every persistent VM
   silently). Failure becomes invisible default; downstream sees
   empty state and assumes "nothing there" rather than "we lost it."

3. **`_ => {}` and unhandled enum variants in protocol matches.**
   `capsem-process/vsock.rs:570` (lifecycle port),
   `capsem-process/vsock.rs:596-645` (handle_guest_msg),
   `capsem-mcp-aggregator/main.rs:106-108`,
   `capsem-process/ipc.rs:306-308`. Adding a variant on one side
   and not the other = silent drop, exactly the
   `StopTerminalStream` symptom we just lived.

4. **No protocol version handshake.** `ServiceToProcess`,
   `ProcessToService`, `HostToGuest`, `GuestToHost` have NO version
   negotiation. Two binaries built across an enum addition silently
   misalign. Today, capsem-service (built 17:30) and the new
   capsem-process (built 18:18+) talked past each other with zero
   diagnostic.

5. **`AppError(StatusCode::INTERNAL_SERVER_ERROR, ...)` returned
   without preceding `tracing::error!`.** ~30+ sites in
   `capsem-service/src/main.rs`. The user gets a 500, the operator
   has nothing in the log to trace back from.

6. **No timing spans on async operations that can hang.**
   `with_quiescence`, `save_state`, `restore_state`, `pause`,
   `fsync(rootfs.img)`, `send_ipc_command`, `wait_for_vm_ready`,
   `attach_disk`, `attach_virtiofs_share`, MCP tool calls. Today we
   wrote and reverted three theories about VirtioFS flush semantics
   because we had no `fsync` duration data. We can't tell if the
   protocol acks fsync without flushing or if the flush actually
   completes.

## Fingerprints (what failure looks like today)

These are real things from this week:

```
# Today: protocol mismatch presents as 30s "guest doesn't respond"
# instead of "decode error: unknown variant 17"
ERROR vsock failed: ...   <-- nothing about decode
INFO  IPC: client connection closed (after 243µs)   <-- swallowed
WARN  handle_suspend (timeout) removing instance

# Today: no fsync duration, so we couldn't tell whether VirtioFS
# was honoring fsync or returning ack without flushing
INFO  capsem-process exited, cleaning up
(no per-step trace; suspend took 8s but only one log line)

# Yesterday from explicit-shutdown-cleanup:
WAL file should be empty after clean shutdown, got 395552 bytes
(no per-stage shutdown trace; we had to add it to even diagnose)

# Loop-device sprint:
EXT4-fs error (device loop0): ext4_lookup:1858: ... iget: checksum invalid
(no host-side trace of when rootfs.img was last written)
```

The common shape: a multi-step pipeline (IPC -> vsock -> guest
agent -> kernel -> disk) fails at one specific step, but the only
log we have is "the whole thing didn't work." We have to add
instrumentation just to start diagnosing, then revert it.

## Scope

**In scope (this sprint, ranked by ROI):**

1. **Codemod `let _ = X.send(...)` on IPC/vsock channels** ->
   `if let Err(e) = X.send(...) { warn!(?e, target = "ipc",
   "send to {channel} failed"); }`. Mechanical, ~50 sites.
   Reuse a small `try_send!` macro in `crates/capsem-core` so we
   don't bloat each call site. Closes pattern (1).

2. **IPC version handshake.** First message on every
   `channel_from_std()` connection is a typed
   `Handshake { version, schema_hash }` (where `schema_hash` is a
   compile-time hash of the enum variant list). Mismatch ->
   `tracing::error!` + close. Same pattern in vsock control bridge
   for HostToGuest/GuestToHost. Closes pattern (3) + (4).
   Implementation note: schema_hash can be a `const` in
   `capsem-proto` computed via build script over the enum source.

3. **Helper `app_error_logged!(level, status, fmt, ...)`** that
   does `tracing::{level}!(...)` *and* returns
   `AppError(status, formatted)` in one move. Codemod the ~30
   service handlers. Closes pattern (5).

4. **Add `#[instrument(skip_all, fields(...))]` to**: `with_quiescence`,
   `save_state`, `restore_state`, `pause`, `attach_disk`,
   `attach_virtiofs_share`, `send_ipc_command`,
   `wait_for_vm_ready`, the rootfs.img fsync block in
   `vsock.rs::Suspend`, MCP tool dispatch. Wrap each with a
   `start = Instant::now()` -> `info!(duration_ms = ...)` if not
   already covered by `tracing::instrument`. Closes pattern (6).

5. **Audit + handle the top 5 `_ => {}` arms with greatest blast
   radius**: lifecycle port, handle_guest_msg, MCP aggregator
   frame loop, ipc.rs main match, vsock control bridge. Each
   becomes `unhandled => warn!(?unhandled, "unknown variant; this
   binary may be older than its peer")`. Closes pattern (3) for
   the active surface; long-tail covered by the handshake.

6. **Codemod `unwrap_or_default()` on result-bearing parses**
   in `mitm_proxy.rs`, `writer.rs`, `registry.rs`. Each becomes
   `unwrap_or_else(|e| { warn!(?e, target = "...", "..."); ... })`
   so the silent fallback is at least audible. Closes pattern (2)
   for the high-blast-radius cases.

7. **Frontend log forwarding hardening.** Replace
   `tauri-log.ts:23` empty catch with a localStorage-buffered
   retry queue + visible "logs not flowing" indicator on the
   toolbar. Same for `api.ts:526` WebSocket onmessage parse
   failure -- log via `log_frontend` and surface on the status
   dock.

**Out of scope:**

- Rewriting any subsystem. This sprint is purely additive logging
  + a thin handshake layer.
- Switching from bincode to a tagged format (msgpack with
  `#[serde(tag = "t")]`). That's a separate sprint; the handshake
  in (2) is enough to detect mismatches without a wire change.
- Per-IP rate limiting / brute-force lockout in
  `capsem-gateway/auth.rs`. Real but separate; tracked in
  audit findings.
- Frontend UX rework (degraded-state icons, error toast catalogs).
  Tracked in `sprints/tray-ui-integration` and
  `sprints/frontend-rebuild`.

**Non-goals:**

- Do NOT add a logging *destination* (no new sinks, no new files).
  Everything routes through the existing `tracing` subscriber so
  it lands in `process.log` / `service.log` and shows up in
  `capsem-doctor` artifact preservation. The point is to write to
  the channels we already have and consume in tooling, not to add
  more.
- Do NOT add `debug!()` for things only a human would care about.
  Default level for new instrumentation is `info!()` for state
  transitions and operations >100ms, `warn!()` for swallowed
  errors that are now logged, `error!()` for auth/IPC/protocol
  failures.
- Do NOT increase log volume in the hot path (per-MITM-byte,
  per-PTY-frame). Sampling or per-connection summary spans only.

## Where to start in a new session

1. Read this file (you already did, by triggering `/dev-sprint`).
2. Read the audit findings index at
   `sprints/observability-stop-the-bleeding/audit-findings.md`
   (TODO: extract from this file's prose summary into a table).
3. Pick the codemod to land first. Recommendation: (1) -- the
   `let _ = X.send(...)` codemod in vsock.rs/ipc.rs. It's the
   single highest-blast-radius change and unblocks the next bug
   investigation immediately.
4. For (1): start with `crates/capsem-process/src/vsock.rs`. The
   `try_send!` macro should live in `crates/capsem-core`'s
   prelude so all process/service/agent crates can use it.
   Cap macro to one log line per failure, use `target = "ipc"` so
   it's filterable.
5. Validate: re-run today's repro (kill capsem-service, rebuild
   capsem-process across an enum addition, capsem_suspend the
   VM). The expectation post-(1+2) is: handshake fires first,
   logs `error!("IPC version mismatch: peer schema_hash X, ours Y")`,
   refuses the connection. Service handle_suspend returns 500
   *with that error* in the response body, not "timed out."

## Repro for the patterns

Patterns (1) + (3) + (4) -- silent IPC mismatch:

```bash
# Build process at HEAD.
cargo build -p capsem-process

# Pretend service is at an older commit by editing
# crates/capsem-proto/src/ipc.rs to remove StopTerminalStream
# (move it to the end of ServiceToProcess so variant numbering
# of earlier variants doesn't shift). Build:
cargo build -p capsem-service

# Restart service, then:
just run "echo hi"   # Suspend any persistent VM.
# Today: hangs ~30s, then "suspend timed out".
# Post-fix: handshake fails fast with structured error.
```

Pattern (5) -- AppError without log:

```bash
# Send a malformed POST to a service handler.
curl -s --unix-socket ~/.capsem/run/service.sock \
  -X POST -d '{"bogus":1}' http://localhost/exec/no-such-vm
# Today: 500 in response, nothing useful in service.log.
# Post-fix: `error!(target="service", id="no-such-vm",
# error="not found", "exec failed")` in service.log.
```

Pattern (6) -- no fsync timing:

```bash
# Suspend a heavy-churn VM (see test_svc_loop_device_after_resume.py).
# Grep process.log for "fsync".
# Today: zero hits.
# Post-fix: `INFO duration_ms=X target=fs op=fsync path=rootfs.img`.
```

## Acceptance criteria

The sprint is done when:

- [ ] `rg 'let _ = .*\.send\(' crates/capsem-process/src/{vsock,ipc}.rs`
      returns zero hits (modulo Drop / cleanup paths that have a
      comment explaining why).
- [ ] Every `channel_from_std::<...>()` site immediately calls
      a `negotiate(...)` helper that exchanges
      `Handshake { version, schema_hash }`. Mismatch produces
      `tracing::error!` and a typed error, not a closed socket.
- [ ] `rg 'AppError\(StatusCode' crates/capsem-service/src/main.rs`
      shows that every site is preceded by a `tracing::{error,warn}!`
      OR uses the new `app_error_logged!` macro.
- [ ] Suspend produces at minimum these structured spans in
      `process.log`: `quiescence`, `apple_vz_pause`,
      `apple_vz_save_state`, `host_fsync_rootfs`, each with
      `duration_ms`.
- [ ] `tests/capsem-service/test_svc_loop_device_after_resume.py`
      either passes (if pattern (4) closure also closes the
      EXT4-on-VirtioFS hot-cache divergence) or fails with a
      *useful* message that points at the right host-side step
      thanks to the new spans.
- [ ] `tests/capsem-service/` gains a regression test that
      forces an enum-variant mismatch and asserts the handshake
      fires + the connection refuses with a logged error
      containing both schema hashes.
- [ ] `just test` passes.

## Why now

Three sprints in a row (vsock-resume-reconnect, explicit-shutdown-
cleanup, loop-device-io-after-resume, plus today's session)
spent the majority of their wall-clock on *adding instrumentation
mid-investigation*, finding the bug, then reverting the
instrumentation. The instrumentation is what's load-bearing; the
bug fix is the cheap part. Land the instrumentation first, keep
it, and the next four sprints close 2-3x faster.

## Related

- `sprints/loop-device-io-after-resume/ISSUE.md` -- depends on this
  sprint to provide fsync timing data.
- `sprints/done/explicit-shutdown-cleanup/ISSUE.md` -- same root
  cause class (silent Drop-time cleanup failures).
- `sprints/done/vsock-resume-reconnect/ISSUE.md` -- same root cause
  class (silent vsock close + reconnect failures).
- Today's commits:
  - `7043dda fix(suspend): three-stage rootfs.img flush + don't claim Suspended on failure`
  - `b86e5fd test(suspend): pin loop-device-io-after-resume bug with failing dmesg check`
  - `867883d fix(service): guest-initiated shutdown -> Stopped, not Defunct`
- The audit transcript that produced this sprint is preserved in
  the session log; the synthesized findings are in the prose
  summary above (extract to `audit-findings.md` as the first task
  if it's useful for tracking individual close-outs).
