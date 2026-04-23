# Tracker: explicit-shutdown-cleanup

## Tasks

- [x] DbWriter: tx/join_handle to std::sync::Mutex<Option<...>>
- [x] DbWriter: add `shutdown_blocking(&self)` + CAPSEM_TEST_SLOW_CHECKPOINT_MS hook
- [x] DbWriter tests: Arc<DbWriter>::shutdown_blocking, idempotent, slow-checkpoint
- [x] FsMonitor: store JoinHandle, add `shutdown_and_join(self)`
- [x] capsem-process: Shutdown struct + signal-handler drain
- [x] `cargo test -p capsem-logger -p capsem-core -p capsem-process` — 88 + 11 + 87 pass
- [x] session-lifecycle Python test under `-n 4` — 16/16 pass incl. WAL test
- [x] Broader regression sweep under `-n 4` — session-lifecycle + cleanup
      + recovery + stress + capsem-service = 159 passed / 4 skipped
- [x] Update CHANGELOG.md
- [x] Update /dev-rust-patterns
- [ ] Commit

## Notes

- The immediate trigger is an intermittent WAL-left-dirty on `DELETE` under
  -n 4 load. `shutdown_lock` made it rare but didn't remove the race.
- capsem-service handle_delete uses `graceful=false`: SIGTERM + 1s poll +
  SIGKILL. Cleanup must finish inside that 1s window.
- FsMonitor already flushes on shutdown_rx/EOF — we just needed to join the
  thread so the caller can sequence "fs_monitor done" before "db done".
- Sibling audit (capsem-service, capsem-gateway, capsem-mcp{,-aggregator,
  -builtin}, capsem-tray): no equivalent Drop-dependent
  durability-critical work. capsem-service uses `kill_on_drop` + parent-
  watch; capsem-gateway has `with_graceful_shutdown`; the mcp crates are
  stdio-line only; capsem-tray does UI polling. None of them own a
  SQLite writer or a notify watcher on a shared session DB. Left alone
  for this sprint; the pattern is now documented in /dev-rust-patterns
  so the next long-running process wires through `Shutdown` from day one.
- Deferred per sprint "out of scope": don't widen the 1s SIGKILL budget
  or remove `shutdown_lock` yet — both can become follow-ups once the
  explicit-cleanup path has bake time.
