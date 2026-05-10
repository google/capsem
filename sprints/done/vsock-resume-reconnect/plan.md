# Sprint plan: hot-swappable vsock bridges

## Context

Full diagnosis lives in [ISSUE.md](./ISSUE.md). This plan is the execution
strategy derived from that diagnosis plus empirical evidence from the
2026-04-22 stress run.

## What we're building

Make `capsem-process`'s vsock layer resilient to Apple VZ's post-resume
connection resets by **accepting continuously** and **hot-swapping** the
underlying fd into the existing reader/writer bridges without dropping
VM state.

## Why

Two failure modes collapse into the same cure:

1. **Pre-handshake EPIPE** (the ISSUE.md 4% tail). Apple VZ hands us a
   half-open control fd; host write of `BootConfig` fails with EPIPE.
   Current code exits the process.
2. **Post-handshake framing desync** (surfaced by the stress run and
   user analysis). Connection resets *after* the handshake leave the
   long-lived control reader trying to decode a length-prefix from
   payload bytes — the infamous `control frame too large (0x81A08329)`
   where `0x81` is a MessagePack map header misread as a length header.
   Current code logs and tears down readiness.

The narrow EPIPE retry (current WIP) only addresses failure #1. A
structural fix — one accept loop for the VM lifetime, with rekeyable
bridges — addresses both and matches the guest's existing reconnect
behaviour.

## Non-goals

- Do not rewrite the capsem-core IPC protocol.
- Do not touch `ipc_tx` / `ctrl_tx` / `JobStore` semantics — transport
  layer only.
- Do not fix the loop-device I/O error surfaced during stress runs
  (`loop: Write error at byte offset 140509184 ... EXT4-fs (loop0):
  failed to convert unwritten extents`). That is a separate guest-side
  bug and deserves its own sprint; hot-swap *masks* it but doesn't
  cure it.

## Key decisions

### Architecture: rekey via mpsc channels

Each bridge thread (control reader, control writer, terminal reader,
terminal writer) owns its current fd plus a receiver for a rekey
channel. On reader EOF/framing error or writer EPIPE, the bridge drops
the bad fd, **clears all partial-read buffers**, and blocks on the
rekey channel for a new fd. The outer accept loop runs the handshake on
each new `CONTROL` connection and sends the accepted fd down the
rekey channels.

This preserves:

- `ipc_tx` and `ctrl_tx` consumers — they never see rekey events.
- `JobStore` — in-flight oneshots stay registered across rekeys.
- Single-writer serialization of the control channel (heartbeat +
  command handler continue to funnel through one writer thread).

### Framing buffer reset is mandatory

Any carry-over buffer content between fds causes exactly the
`0x81A08329` desync. The reader bridge **must** allocate fresh
`len_buf: [u8; 4]` and `payload: Vec<u8>` on each rekey. No
`BufReader::buffer().consume()` games.

### Retry classification is widened at the bridge, not the handshake

Previous WIP classified `BrokenPipe | ConnectionReset` as retryable.
For the bridge-level hot-swap, **any** read/write error means "drop
this fd and wait for a new one"; genuine guest death is detected by
timeout on the rekey channel (bounded by the service's 30s
`.ready` poll — no new dedicated timer needed).

### Reference the stash, do not pop it

`stash@{0}` has a sound 4-bridge sketch but deleted 11 must-preserve
features (see ISSUE.md §"Deletions to explicitly preserve"). Read it
for the rekey pattern, then re-implement additively on current code.

## The 11 features that must survive (from ISSUE.md)

Gate: after each milestone, re-check this list.

1. 10-second heartbeat (`HostToGuest::Ping { epoch_secs }`) — MITM
   cert validation needs clock sync.
2. `TerminalResize` handler forwarding `cols/rows` to guest.
3. Lifecycle port (`VSOCK_PORT_LIFECYCLE`) for
   `ShutdownRequest` / `SuspendRequest`.
4. Audit port (`VSOCK_PORT_AUDIT`) — length-prefixed MessagePack
   `AuditRecord` → `audit_events` table. Both deferred + dynamic paths.
5. Exec `duration_ms` tracking via `exec_start_times: HashMap<u64, Instant>`.
6. Apple VZ main-thread dispatch for `vm.stop()` and
   `pause()`+`save_state()` via `apple_vz::run_on_main_thread`.
7. `fsync` after `save_state` (closes the `.vzsave` flush race).
8. Error-path `Unfreeze` if save_state fails.
9. `deferred_conns` processing for SNI/MCP/AUDIT arriving ahead of
   control/terminal.
10. Handshake on `spawn_blocking` (don't block the tokio runtime).
11. Reader-break poisoning via `JobStore::fail_all` so in-flight
    callers fail fast instead of 30s timeout.

## Files

Primary:

- `crates/capsem-process/src/vsock.rs` — all structural changes land
  here. The file is 860+ lines today; this will probably add 100-150
  net after merging the rekey plumbing.
- `crates/capsem-process/src/vsock/tests.rs` — new sibling test file
  (per CLAUDE.md: "Rust tests live in a sibling tests.rs"). Existing
  inline tests migrate here.
- `crates/capsem-process/src/main.rs` — unchanged, but its
  `std::process::exit(1)` on `setup_vsock` Err becomes rarer (only
  fires if the rekey system itself fails, e.g. `vsock_rx` closes).

Reference only (no changes):

- `crates/capsem-agent/src/main.rs` — guest reconnect loop (confirmed
  working).

## Milestones

Five commits, each self-contained, each with changelog and tests
green at that point.

### M1 — Land scaffolding + unit tests (WIP, ready to commit)

Extract `perform_handshake`, `collect_terminal_control_pair`,
`is_retryable_handshake_error`. Wrap in `handshake_with_retry`
with narrow EPIPE/ConnectionReset classification. 7 new unit tests,
15 total passing. Does not fix the stress regression but builds
the helpers the hot-swap layer will reuse.

Commit: `fix(process): extract handshake helpers with narrow EPIPE retry`

### M2 — Continuous accept + control rekey channel

Replace the one-shot handshake prologue with a long-lived tokio task
that loops over `vsock_rx.recv()` for the VM's lifetime. On CONTROL
connection: run `perform_handshake`; on success, push the fd down
`control_rekey_tx`. On TERMINAL: push down `terminal_rekey_tx`.
`deferred_conns` logic unchanged. Ready-sentinel still only on
first successful handshake.

Commit: `feat(process): continuous vsock accept with rekey channels`

### M3 — Rekeyable control writer and heartbeat

Single writer thread holds `Option<File>` for the control fd; a
`select!` or channel-join receives either (a) messages to write, or
(b) a new fd from `control_rekey_rx`. Heartbeat thread unchanged —
it still funnels through `ctrl_write_tx`. On write failure, drop
the fd and wait for rekey.

Commit: `feat(process): rekeyable control writer survives fd swaps`

### M4 — Rekeyable control reader with framing reset

Reader blocks on `clone_fd(current_fd)` for length+payload. On any
error (EOF, BrokenPipe, ConnectionReset, framing), drop fd, **reset
all buffers**, receive next fd from `control_rekey_rx`, resume.
`js_for_teardown.fail_all` only fires if `control_rekey_rx` itself
closes (guest truly gone).

Commit: `feat(process): rekeyable control reader with buffer reset`

### M5 — Rekeyable terminal bridge

Terminal reader and writer follow the same rekey pattern. The
coalesce buffer must NOT carry bytes across rekey boundaries —
`CoalesceBuffer::reset()` on swap (add the method if absent).

Commit: `feat(process): rekeyable terminal bridge`

Each milestone runs `cargo test -p capsem-process` + `cargo clippy -p
capsem-process`. The full stress harness runs at the end of M5.

## Done criteria

- [ ] `cargo clippy -p capsem-process` clean.
- [ ] `cargo test -p capsem-process` passes (85+ tests).
- [ ] `just test` passes.
- [ ] `CAPSEM_STRESS=1 uv run pytest tests/capsem-mcp/test_stress_suspend_resume.py -n 8 --tb=line -q` → 50/50 (or at minimum no framing-desync failures; remaining failures must be the separate loop-device I/O issue).
- [ ] All 11 must-preserve features exercised — spot check via
  `just run "capsem-doctor"` and targeted mcp_call traces.
- [ ] `CHANGELOG.md` has an `Unreleased/Fixed` entry per commit.
- [ ] ISSUE.md `[stash@{0}]` is reviewed; stash either popped-and-discarded
  (after extraction) or kept with a note why. Do not leave silently.

## Risks

- **tokio + blocking vsock I/O**. The control reader today uses sync
  reads on `std::fs::File` inside `spawn_blocking`. Rekey semantics
  via channels are trivial in async land; bridging a sync reader to
  an async rekey channel needs either `std::sync::mpsc` + blocking
  recv (OK: reader is already on a dedicated thread) or `tokio::sync::mpsc`
  with `try_recv` polled between reads (less clean). Default to the
  former; flag if it forces multiple copies.
- **Buffer carry-over bugs**. The failure signature
  `control frame too large (0x81A08329)` shows exactly what happens if
  we miss a buffer reset. Each rekey path gets an explicit test.
- **Deadlock on rekey-during-write**. Writer thread sits blocked in
  `file.write_all()`; rekey event cannot preempt. Mitigation: writer
  sets write timeout (nonblocking fd + `select!`) or accepts that a
  wedged write blocks until the fd breaks (which it will, because
  the guest has reset). Prefer the latter — simpler, and we're already
  robust to that via "write fails → drop fd → wait for rekey."

## Out of scope, noted for follow-up

- **Loop-device I/O errors after resume** (5/50 failures in the last
  stress run). Symptom: `loop: Write error at byte offset 140509184`
  + `EXT4-fs failed to convert unwritten extents`. Hot-swap masks the
  symptom but not the cause — the guest's persistent block device
  genuinely corrupts on some save/restore cycles. Separate sprint.
