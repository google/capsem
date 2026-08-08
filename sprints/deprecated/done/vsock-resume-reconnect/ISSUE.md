# Sprint: vsock resume-reconnect

## TL;DR

Apple VZ gives us a **half-open vsock control connection ~4% of the time**
after `save_state` / `restore_state`. The guest agent already reconnects
in a loop when its fd breaks; the host has no matching accept-and-retry
logic, so the first handshake attempt fails with EPIPE and the VM is
permanently unrecoverable until SIGTERM.

Current state (after the 2026-04-22 session) is: the failure is now
clean, observable, and fails fast. What's left is to make it
**self-healing**.

## Measured state as of commit `9d16d7d` (HEAD at handoff time)

- 48/50 pass on the stress repro (`tests/capsem-mcp/test_stress_suspend_resume.py`).
- 2/50 failures, always the same symptom:
  - Host `process.log`: `vsock failed: restore BootConfig write failed: Broken pipe (os error 32)`
  - Guest `serial.log`: `control channel error: Connection reset by peer` → reconnect → `failed to send BootReady: Broken pipe`.
- With the "cheap fix" (`std::process::exit(1)` on setup_vsock Err),
  failures now surface in <1s instead of 30s.

## The full problem, diagnosed

### Apple VZ resume semantics

Pre-suspend:
- Host holds vsock fd A (terminal) + fd B (control).
- Guest holds matching fds a, b on its side.
- `save_state` persists the VM memory + vCPU state, including guest-kernel
  socket state.
- capsem-process exits after save_state → host fds A, B close.
- Guest's a, b are left in a kernel-level state expecting the peer.

On resume (new capsem-process):
- New vsock listeners registered on ports 5000-5005.
- Guest kernel wakes up. First read/write on a or b returns
  `ECONNRESET` (peer is gone). Agent catches this in the bridge loop.
- Agent calls `vsock_connect_retry(VSOCK_HOST_CID, VSOCK_PORT_TERMINAL, ...)`
  and the same for control. New fds established.
- Agent sends `Ready` on new control fd.
- **Sometimes** (~4%), this new control fd is **half-open**: guest-to-host
  direction works (host reads `Ready`), host-to-guest direction is already
  broken (host's `write(BootConfig)` fails with EPIPE).

This is almost certainly an Apple VZ issue — the new vsock connection is
accepted by the listener but the backing kernel state is still tied to
the stale pre-suspend connection. We haven't dug into the framework to
prove it; `dev-rust-patterns` and VZ docs might have hints.

### Why this manifests as a 30s timeout pre-cheap-fix

The previous behavior:
1. Handshake fails inside setup_vsock → setup_vsock returns Err
2. The spawn in `capsem-process/src/main.rs:424` logs the error and the
   spawned task exits silently
3. capsem-process itself keeps running — no exit, no signal
4. Service polls `.ready` (never created because handshake failed) for 30s
5. Returns `exec-ready timeout` with no specific diagnosis

The "cheap fix" at `main.rs:446` now calls `std::process::exit(1)` so
service's child-exit handler reclaims the instance in <1s.

### The structural fix

Host-side reconnection support matching the guest's loop:

1. After handshake fails with EPIPE (or any transient error), don't exit
   the vsock task. Wait for a NEW control-port accept.
2. When the guest's reconnect logic produces a fresh connection, attempt
   handshake again on the new fd.
3. Cap retries (e.g. 3 attempts over ~5 seconds) so we don't loop forever
   on a genuinely broken VM.
4. Post-handshake reader/writer bridges must also be re-keyable: when the
   current fd errors out mid-session, splice in the next one from the
   reconnect stream.

This is exactly what the stashed refactor was attempting. It got
rejected originally because it silently dropped features (heartbeat,
audit port handling, lifecycle port, Apple VZ main-thread dispatch for
pause/save_state, fsync after save_state, error-path Unfreeze). The
plan is to **re-do the refactor additively** on top of current code,
preserving every existing feature, just layering rekey-capable bridges
underneath.

## Stash reference

The exploratory reconnection refactor is stashed:

```
stash@{0}: On next-gen: vsock-reconnect-refactor-wip
```

To view: `git stash show -p stash@{0}`
To pop:  `git stash pop stash@{0}`

The stash has a structurally-sound 4-bridge layout (terminal, control
reader, control writer, central dispatcher) with `control_rekey_tx` /
`terminal_rekey_tx` channels. Useful as a design reference even if you
don't pop it — the new implementation should adopt the rekey pattern
but not the deletions.

## Deletions to explicitly preserve when redoing

A diff-level comparison against the stash identified these features
that must survive any refactor:

1. **10-second heartbeat thread** sending `HostToGuest::Ping { epoch_secs }`
   — commit `650f7d3` wired this specifically for "MITM cert validation
   failure from guest clock drift". Cannot lose.
2. **TerminalResize handler** (`ServiceToProcess::TerminalResize { cols, rows }`
   → `HostToGuest::Resize`) — terminal sizing from tray/browser.
3. **Lifecycle port** (`VSOCK_PORT_LIFECYCLE`) handling for
   `GuestToHost::ShutdownRequest` / `SuspendRequest` — commit `28bde20`
   ("make capsem-process exit on SIGTERM on macOS").
4. **Audit port** (`VSOCK_PORT_AUDIT`) — reads length-prefixed MessagePack
   audit records, writes to session.db's audit_events. Security
   telemetry. Both deferred-connection and dynamic paths.
5. **Exec `duration_ms` tracking** via `exec_start_times: HashMap<u64, Instant>`
   → `ExecEventComplete.duration_ms`. Session-DB exec timing relies on
   this being non-zero.
6. **Apple VZ main-thread dispatch** for `vm.stop()` on Shutdown and for
   `pause()`+`save_state()` on Suspend — commit `8ee332a` gated these
   behind `#[cfg(macos)]` via `apple_vz::run_on_main_thread` because VZ
   asserts CFRunLoop.
7. **`fsync` after `save_state`** — commit `3ccdce9` closed a race where
   the next resume read a not-yet-flushed `.vzsave`.
8. **Error-path `Unfreeze`** in the Suspend handler — if save_state
   fails, the guest stays frozen unless we send Unfreeze.
9. **`deferred_conns` processing** — SNI_PROXY / MCP_GATEWAY / AUDIT
   connections that race ahead of terminal/control during initial
   wait must be processed, not discarded.
10. **Handshake on `spawn_blocking`** — commit `9735076` (this sprint)
    moved handshake reads/writes off the async worker. Whatever the
    new rekey loop does must also not block the runtime.
11. **Reader-break poisoning** — `JobStore::fail_all` resolves pending
    oneshots with `JobResult::Error` when the reader dies so callers
    don't hang 30s. Must still fire when a bridge gives up for real.

## Where to start in a new session

1. Read this file (you already did, by recommending `/dev-sprint`).
2. Read the test: `tests/capsem-mcp/test_stress_suspend_resume.py`.
   This is the stress repro (50× the real test, xdist-distributable).
3. Read current `crates/capsem-process/src/vsock.rs::setup_vsock` and
   note how it's structured today.
4. `git stash show -p stash@{0}` to see the earlier exploration.
5. Decide: refactor `setup_vsock` to a rekey-capable bridge layout OR
   wrap the existing handshake in a retry-on-EPIPE loop (smaller
   surgical fix). Probably retry-on-EPIPE is enough for the 4% tail,
   assuming Apple VZ eventually gives us a fully-open connection on
   retry.

## Open questions worth empirical answers

- Does a *second* handshake attempt on a fresh accept succeed when
  the first fails with EPIPE? (Cheap to test: add a retry loop
  in setup_vsock, rerun the stress harness, see if the 2/50 drops
  to 0/50.) If yes, we might not need the structural refactor at
  all — a tight retry loop would be sufficient.
- Does the EPIPE happen deterministically on the first accept after
  resume, or sporadically during the handshake write itself? The
  logs suggest the former but we haven't re-ordered to prove it.
- Is the Apple VZ vsock behavior documented anywhere (developer
  forums, radar tickets)? Worth 20 minutes of searching before
  committing to a design.

## Repro

```bash
# Assumes binaries are built (just _sign or cargo build -p capsem-process)
uv run pytest tests/capsem-mcp/test_stress_suspend_resume.py \
    -n 8 --tb=line -q
```

Currently: 48/50 pass. Expected after structural fix: 50/50 pass.

## Related commits from this session

- `f225a8c` — fix(build): clean up install-test container on any exit path
- `e4b0e85` — fix(build): durable disk cushion preflight for test-install
- `6c513ec` — fix(tests): resilient artifact preserver (copytree → os.walk)
- `325e9f5` — fix(service): align wait_for_vm_ready with project backoff convention
- `9735076` — fix(process): harden vsock handshake + recovery from reader wedge
- (HEAD) — fix(process): exit on setup_vsock failure so service sees the failure promptly
