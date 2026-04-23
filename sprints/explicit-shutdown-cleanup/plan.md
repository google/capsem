# Plan: signal-driven explicit cleanup for background-thread owners

## Goal

Make VM teardown cleanup deterministic. Stop relying on tokio-runtime-drop
ordering to finish the DbWriter WAL checkpoint and FsMonitor flush inside
capsem-process's signal window, so `session.db-wal` is reliably empty after
`DELETE /delete/{id}` (the `test_wal_absent_after_clean_shutdown` regression).

## Strategy

Two concrete, minimal building blocks in the shared crates, then wire them
through the one process that has the bug today (capsem-process).

1. **DbWriter::shutdown_blocking(&self)** — synchronous, idempotent, Arc-safe.
   Takes the stored sender out, joins the writer thread. Writer thread drains
   remaining ops and runs `PRAGMA wal_checkpoint(TRUNCATE)`. Safe to call from
   any thread while other Arc<DbWriter> clones exist — subsequent writes are
   no-ops. Existing Drop impl delegates to it.

   Supporting change: convert `tx: Option<Sender>` to
   `tx: std::sync::Mutex<Option<Sender>>` so `&self` can take it. Same for
   `join_handle`. `write()` clones the sender under the lock and releases the
   lock before `.await`. No hot-path regression (Sender clone is cheap Arc).

   Test hook: `CAPSEM_TEST_SLOW_CHECKPOINT_MS` inserts a sleep before the
   final checkpoint so a unit test can assert explicit cleanup waits for it
   even when implicit drop would not.

2. **FsMonitor::shutdown_and_join(self)** — consume the monitor, signal the
   event loop, join its thread. The event loop already flushes on shutdown_rx
   tick; this just makes the join visible so the caller can sequence it.

## Wiring in capsem-process

Introduce a `Shutdown` struct owned by `main()`:

```rust
struct Shutdown {
    db: Option<Arc<DbWriter>>,
    fs_monitor: Option<FsMonitor>,
}
```

`run_async_main_loop` populates it after constructing the owners. The SIGTERM
handler:

1. locks the shutdown mutex,
2. drains `fs_monitor` first (fs_events fan into DbWriter — flush first),
3. drains `db` (via spawn_blocking, since shutdown_blocking joins a thread),
4. calls `CFRunLoopStop`.

Cleanup runs on a tokio worker. Main thread is blocked in `CFRunLoopRun`
until step 4. Runtime drop after main returns is now deterministic because
the heavy work already happened.

## Out of scope for this sprint

- Widening the 1s SIGKILL budget in handle_delete / handle_purge.
- Replacing DbWriter architecture.
- Removing shutdown_lock (host-serialization stays; it solves a different
  problem).
- Full audit of capsem-gateway / capsem-mcp-aggregator / capsem-tray. Those
  get a follow-up note in /dev-rust-patterns but no code changes yet — we
  want the capsem-process fix verified under full test load before touching
  the others.

## Files to modify

- `crates/capsem-logger/src/writer.rs` — tx/join_handle to Mutex<Option<...>>,
  add shutdown_blocking, add CAPSEM_TEST_SLOW_CHECKPOINT_MS hook.
- `crates/capsem-logger/src/writer.rs` tests — cover shutdown_blocking called
  through Arc, idempotent, WAL clean after slow-checkpoint hook.
- `crates/capsem-core/src/fs_monitor.rs` — store JoinHandle, add
  shutdown_and_join.
- `crates/capsem-process/src/main.rs` — Shutdown struct, drain in SIGTERM
  handler before CFRunLoopStop, thread it through run_async_main_loop.
- `skills/dev-rust-patterns/SKILL.md` — document the pattern next to the
  host-serialization pattern.
- `CHANGELOG.md` — Fixed entry for this sprint.

## Done when

- `cargo test -p capsem-logger` passes, including new shutdown_blocking test.
- `cargo test -p capsem-process` passes.
- `tests/capsem-session-lifecycle/test_wal_cleanup.py` passes under `-n 4`
  (the failing case) and in isolation.
- `skills/dev-rust-patterns/SKILL.md` has a "Signal-driven explicit cleanup"
  section.
- CHANGELOG updated under `## [Unreleased]`.
