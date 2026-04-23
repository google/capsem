# Sprint: signal-driven explicit cleanup for background-thread owners

## TL;DR

Every long-running Capsem process today relies on **Rust `Drop` + tokio
runtime drop** for shutdown cleanup. When a SIGTERM arrives,
capsem-process (and similarly structured siblings) calls `CFRunLoopStop`
to unblock the main loop, `main` returns, the tokio runtime drops, all
tasks abort, and — we hope — every `Drop` impl finishes its business
before the OS reaps us.

Under load, that "we hope" breaks. `DbWriter::Drop` holds a `join()` on
its writer thread, which is finishing a batch and then running
`PRAGMA wal_checkpoint(TRUNCATE)`. `FsMonitor::Drop` quiesces the
notify watcher. Apple VZ's `stop` has its own teardown. The service's
`handle_delete` fast path gives the whole process **1 second** before
SIGKILL. Under N concurrent teardowns on one host, these steps starve
each other of the main thread + APFS + virtiofs bandwidth they need,
and a single shutdown can blow the 1s budget. The bug-review pass that
added `shutdown_lock` (`e485126` + follow-ups, 2026-04-23) makes this
less frequent by serializing teardowns — but the race is still there;
we've just thinned its probability.

The real fix is the **signal-driven explicit-cleanup pattern** for
background-thread owners: on SIGTERM, drive owned background resources
to completion synchronously, *then* signal the main loop to exit.
Don't rely on drop order or runtime-drop-aborts-tasks semantics.

This sprint:
1. Builds the pattern (helper or just a convention) in
   `capsem-process` first, where we know the bug lives.
2. Audits the other Rust processes (`capsem-service`, `capsem-gateway`,
   `capsem-mcp`, `capsem-mcp-aggregator`, `capsem-mcp-builtin`,
   `capsem-tray`) for equivalent background-thread owners that today
   rely on Drop.
3. Documents the pattern in `/dev-rust-patterns` alongside the
   host-serialization pattern that was added in the same bug review.

## Fingerprint

The immediate trigger is `test_wal_absent_after_clean_shutdown`:

```
tests/capsem-session-lifecycle/test_wal_cleanup.py:38:
AssertionError: WAL file should be empty after clean shutdown, got 395552 bytes
```

Observed once on a 2026-04-23 `just test` run, worker `gw0`, co-failing
with `test_empty_bearer_returns_401` on worker `gw1`. Neither
reproduces in isolation or at `-n 4` in a clean tree. Both require the
full test load -- i.e., the kind of concurrency where the 1s fast-path
SIGKILL budget actually gets blown.

The WAL is only the most visible symptom. Other equally-at-risk
resources:

- `capsem_logger::DbWriter` — writer thread + WAL checkpoint on Drop.
- `capsem_core::fs_monitor::FsMonitor` — watcher thread (notify-rs
  backend). Drop quiesces it. If the last events aren't delivered,
  fs_events rows don't land in `session.db`.
- MITM proxy telemetry — `ai_traffic` records can be buffered in a
  task that drops with pending writes.
- `capsem-mcp-aggregator` — holds stdio handles to external MCP
  servers. Ungraceful exit leaves those processes re-parented to init
  momentarily before `capsem-guard`'s parent-watch reaps them.
- Anything that owns a `tokio::sync::mpsc::Sender` fan-out where the
  receiver does a drain-and-commit on EOF (the `writer_loop` pattern).

## Current state after `shutdown_lock`

`shutdown_lock` (the host-serialization pattern that shipped with the
bug review) forces one teardown at a time through `shutdown_vm_process`.
That gives each teardown the full 1s budget alone on the host, which
in practice will make the WAL bug rare. But:

- The 1s budget is still there; any single teardown that slows down
  for an unrelated reason (APFS fsync latency spike, VZ teardown
  getting slower in a future macOS, writer thread with 128+ pending
  ops) still SIGKILLs mid-checkpoint.
- `capsem-mcp-aggregator` / `capsem-gateway` / `capsem-tray` shutdowns
  don't pass through `shutdown_lock` at all -- the lock only covers
  VM-process teardown orchestrated by `capsem-service`.
- This is a `Drop`-based contract for something that should be an
  explicit signal-handler contract.

## What "the pattern" should look like

Current (capsem-process, simplified):

```rust
rt.spawn(async move {
    let mut sigterm = signal(SignalKind::terminate()).unwrap();
    sigterm.recv().await;
    // Just stops the run loop. Everything else is implicit Drop.
    CFRunLoopStop(CFRunLoopGetMain());
});
CFRunLoopRun();
// main returns -- rt drops -- tasks abort -- Drops run -- process exits.
// There is no deterministic "checkpoint finished before exit" guarantee.
```

Proposed:

```rust
// Own the background-thread resources explicitly at a level the signal
// handler can see. A Shutdown struct collects `Drop`-able handles that
// the handler will drain *before* letting main return.
struct Shutdown {
    db: Option<Arc<DbWriter>>,       // will be dropped and joined
    fs_monitor: Option<FsMonitor>,
    // ...other background-thread owners
}

let shutdown = Arc::new(tokio::sync::Mutex::new(Shutdown { ... }));

let shutdown_for_sig = Arc::clone(&shutdown);
rt.spawn(async move {
    let mut sigterm = signal(SignalKind::terminate()).unwrap();
    sigterm.recv().await;

    // Drain in known order. Each `.take()` + `drop()` runs the Drop
    // synchronously and waits for the background thread to finish.
    let mut s = shutdown_for_sig.lock().await;
    drop(s.fs_monitor.take());   // flush fs_events first
    drop(s.db.take());            // then checkpoint WAL
    // ...then stop the run loop
    CFRunLoopStop(CFRunLoopGetMain());
});
```

Key properties:

1. **Deterministic order.** fs_events fans out through DbWriter; flush
   fs_monitor first so its pending writes are seen by DbWriter before
   DbWriter's Drop runs the checkpoint. (Current code relies on
   reverse-declaration-order Drop to get this right, which is fragile.)
2. **Synchronous join.** No "hope the task finishes in time" -- the
   handler runs until each drop completes.
3. **Signal handler owns the budget, not the parent's SIGKILL timer.**
   The caller (service's `shutdown_vm_process`) can still enforce an
   outer bound, but the inner signal handler is now in charge of
   getting cleanup right.
4. **Applicable to every Rust process with worker threads.** Same
   shape in `capsem-gateway`, `capsem-service`, `capsem-mcp-*`,
   `capsem-tray`.

## Where to start in a new session

1. **Land the pattern in `capsem-process` first.** The WAL bug lives
   here and it has the most background-thread surface
   (`DbWriter`, `FsMonitor`, VZ handle, deferred vsock connections).
   See `crates/capsem-process/src/main.rs` signal branch and
   `run_async_main_loop` — the goal is to pull owned resources up
   where the signal handler can drop them.
2. **Add a unit test that fails under the *current* 1s SIGKILL
   behavior.** The test from `tests/capsem-session-lifecycle/
   test_wal_cleanup.py` is almost right but doesn't blow the 1s
   budget reliably in isolation. Options:
   - Instrument DbWriter to artificially delay the checkpoint by
     `CAPSEM_TEST_SLOW_CHECKPOINT_MS` and set that to 2000 in the
     test; without explicit-cleanup the WAL should stay dirty; with
     it, empty.
   - Or a capsem-process unit test that runs with an in-memory
     simulation of the shutdown order.
3. **Audit sibling processes.**
   - `capsem-gateway`: already has `with_graceful_shutdown`, but
     cleanup is implicit. What does it own? AuthState has a cleanup()
     method but is it always called?
   - `capsem-service`: on SIGTERM kills companions explicitly, but
     its own `ServiceState` drops contain
     `tokio::sync::Mutex<magika::Session>`, registry writes, etc.
   - `capsem-mcp` + `capsem-mcp-aggregator` + `capsem-mcp-builtin`:
     stdio line buffers, session log handles, subprocess ownership.
   - `capsem-tray`: thread that polls the gateway status.
4. **Decide: helper crate or convention.** Two options, pick one:
   - **Convention:** each binary exposes its own `Shutdown` struct
     and drains it in the signal handler. No crate-level dependency,
     but every process needs the same boilerplate.
   - **`capsem-core` helper:** a `GracefulShutdown` or
     `BackgroundOwners` primitive others plug into. Lives next to
     `capsem-guard`. Harder to iterate per-binary, but consistent.
   - Probably start with convention; extract to a helper if/when
     three processes have the same shape.
5. **Document in `/dev-rust-patterns`.** The skill already has the
   host-serialization pattern (`save_restore_lock`, `shutdown_lock`).
   Add a sibling section "Signal-driven explicit cleanup for
   background-thread owners" with an example and a checklist of
   places to apply it.
6. **Follow up on `shutdown_lock`'s cons.** Once explicit-cleanup is
   in place, the 1s SIGKILL budget can be widened back to something
   more forgiving (or kept at 1s -- by then it won't matter because
   cleanup is deterministic, not best-effort). `handle_purge`'s
   `join_all` can go back to concurrent shutdowns safely, since each
   capsem-process is now responsible for its own clean exit within
   its own signal handler.

## Repro

Today, no reliable local repro. Investigation should start by making
one — a test or harness that lets you dial up the checkpoint latency.

## Scope

**In scope:**
- Signal-driven explicit cleanup in `capsem-process` for `DbWriter`
  and `FsMonitor` at minimum.
- Audit + (where justified) apply the same pattern in the other Rust
  binaries.
- Test that asserts WAL is clean even under an artificially slow
  checkpoint.
- Docs: `/dev-rust-patterns` gets the new pattern described.
- Follow-up adjustment to `shutdown_lock` or fast-path timeouts once
  the explicit-cleanup landing lets them relax.

**Out of scope:**
- Replacing the DbWriter architecture (channel → dedicated thread).
  That's a bigger refactor and orthogonal to cleanup-on-shutdown.
- Replacing `capsem-guard` or the parent-watch primitive.
- Changing the service's SIGTERM/SIGKILL budget permanently; that's
  a follow-up once this sprint's changes are stable.

## Non-goals

- Do **not** remove `shutdown_lock` as part of this sprint.
  Host-serialization and explicit-cleanup solve different problems;
  keep both until we have evidence that either is redundant. See
  `/dev-rust-patterns` "Concurrency patterns" section for the
  distinction.
- Do **not** change `handle_purge` back to concurrent teardowns inside
  this sprint -- that's a cleanup-is-deterministic follow-up.
