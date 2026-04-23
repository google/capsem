---
name: dev-rust-patterns
description: Rust patterns and lessons learned in Capsem. Use when writing Rust code for capsem-core, capsem-app, or capsem-agent. Covers async/tokio patterns, non-blocking I/O, cross-compilation gotchas, error handling, and hard-won lessons from past bugs. Read references/rust-async-patterns.md for the full tokio reference.
---

# Rust Patterns

## Async / non-blocking

Capsem uses tokio for all async I/O. The MITM proxy, vsock manager, file monitor, and auto-snapshot scheduler are all async.

### Never block the tokio runtime

Long-running synchronous work (FUSE request processing, disk I/O, compression) must run on a dedicated thread via `tokio::task::spawn_blocking` or a dedicated `std::thread`. Blocking inside a tokio task starves other tasks.

The VirtioFS FUSE server runs on its own thread for this reason -- FUSE ops are synchronous by nature (read, write, lookup) and can't be made async without significant complexity.

### Blocking-in-async anti-pattern (systemic -- audit, don't spot-fix)

Any code path that does blocking I/O inside an async function or while holding a `tokio::sync::Mutex` is a bug. This causes the tokio worker thread to stall, freezing the entire gateway, UI, or network stack until the blocking operation completes.

**What counts as blocking I/O:**
- `std::process::Command` (subprocess execution)
- `std::fs::*` (read, write, copy, remove_dir_all, create_dir_all)
- `walkdir::WalkDir` (directory traversal)
- `blake3::Hasher` on large data (hash computation)
- `std::thread::sleep`

**The fix pattern** -- same as `call_mcp_tool` in `crates/capsem-app/src/commands/mcp.rs`:
```rust
let result = tokio::task::spawn_blocking(move || {
    let rt = tokio::runtime::Handle::current();
    rt.block_on(async {
        let mut guard = mutex.lock().await;
        sync_blocking_work(&mut guard)
    })
}).await.unwrap_or_else(|e| /* handle panic */);
```

**Known fixed sites (2026-03-27):** MCP gateway file tool dispatch (gateway.rs), auto-snapshot timer (vsock_wiring.rs), asset hash verification (asset_manager.rs). If you add new file tools or snapshot operations, use the same `spawn_blocking` pattern.

### Channel patterns

- `tokio::sync::mpsc` for producer-consumer (vsock data flow, telemetry events)
- `tokio::sync::broadcast` for fan-out (serial output to multiple subscribers)
- `tokio::sync::oneshot` for single-response request-reply (control messages)

### Coalescing buffer

Terminal output uses a `CoalesceBuffer` (8ms window, 64KB cap) to batch small vsock reads into larger writes. This prevents xterm.js from choking on thousands of tiny updates. The pattern: accumulate into a buffer, flush on timer or size threshold.

### Graceful shutdown

Use `tokio::select!` with a cancellation token or shutdown signal. Every long-running task must respect shutdown. Dangling tasks after VM exit cause resource leaks.

## Cross-compilation

Guest binaries target `aarch64-unknown-linux-musl` and `x86_64-unknown-linux-musl`. Key gotchas:

- **Platform-specific types**: `libc::ioctl` request param is `c_ulong` on macOS but `c_int` on Linux. Use `as _` to let the compiler infer the correct type.
- **Linker**: `.cargo/config.toml` sets `linker = "rust-lld"` for both musl targets.
- **No std dependencies**: musl builds are fully static. Avoid crates that link to system libraries.
- **Test on both**: `cargo check --target aarch64-unknown-linux-musl` catches cross-compile errors without needing to boot a VM.

## Error handling

- Use `anyhow::Result` for application code (capsem-app, scripts)
- Use `thiserror` for library errors in capsem-core (typed, matchable)
- Propagate errors up, don't swallow them. If a function returns `Result`, the caller must handle it.
- Log errors at the point where you have context, then propagate. Don't log AND propagate (causes duplicate log lines).

## Bidirectional I/O -- thread per direction

When bridging two blocking file descriptors bidirectionally (e.g., TCP socket to vsock in `net_proxy.rs`, or master PTY to vsock in `capsem-pty-agent`), doing both reads and writes in a single thread using `poll(2)` causes deadlocks. If both outgoing buffers fill simultaneously, a single thread blocks on writing and stops reading, creating mutual lockup. Always spawn a dedicated thread for at least one direction (`std::thread::spawn` for `fd_b -> fd_a` while the main thread handles `fd_a -> fd_b`).

## Serde -- avoid `serde_json::Value` on LLM payloads

The MITM proxy and ai_traffic parsers handle massive HTTP payloads (megabytes of tool calls, histories, images). Parsing these into `serde_json::Value` does full DOM allocation, which is inefficient and risks memory exhaustion.

**Rules:**
- Define targeted structs with `#[derive(Deserialize)]`. Serde skips and discards fields not in the struct without allocating memory for them.
- For struct fields that hold large, unconstrained JSON (tool call arguments, function responses, full model outputs) and are only converted to strings: use `Box<serde_json::value::RawValue>` instead of `serde_json::Value`. `RawValue` keeps the JSON as an unparsed string slice -- zero DOM allocation. Access the raw JSON string via `.get()`.
- Never add `serde_json::Value` fields to structs that parse LLM request/response bodies. If you only need a string representation, use `RawValue`. If you need to traverse nested fields, use a typed struct.
- Remove unused fields from deserialization structs -- they still force Serde to allocate.

**Example -- before (bad):**
```rust
struct FunctionCall {
    name: Option<String>,
    args: Option<serde_json::Value>,  // full DOM parse of potentially huge args
}
// later: let arguments = fc.args.as_ref().map(|v| v.to_string());
```

**After (good):**
```rust
struct FunctionCall {
    name: Option<String>,
    args: Option<Box<serde_json::value::RawValue>>,  // zero-copy string slice
}
// later: let arguments = fc.args.as_ref().map(|v| v.get().to_owned());
```

## Memory and resource management

- **File handle limits**: VirtioFS caps at 4096 open file handles, returns `EMFILE` beyond that.
- **Read size limits**: VirtioFS clamps reads to 1MB, gather buffers to 2MB.
- **Safe deserialization**: `read_struct` returns `Option<T>` with bounds checks in all builds (not just debug).
- **irqfd for interrupt delivery**: Guest interrupt signaling uses `irqfd` to avoid cross-thread syscall overhead.

## Concurrency patterns

- **RwLock for caches**: Cert authority uses `RwLock<HashMap>` -- many readers, rare writers. Use `read()` first, upgrade to `write()` only on cache miss.
- **Arc for shared state**: VM state, proxy config, and telemetry handles are `Arc`-wrapped for sharing across tasks.
- **Per-connection tasks**: The MITM proxy spawns a new tokio task per connection. Each task owns its TLS state and upstream connection. No shared mutable state between connections.

### Host-serialization locks for per-host critical sections

When a service orchestrates N sibling child processes on a single host and some operations cannot safely run two-at-a-time on that host -- whether because of a framework constraint (Apple VZ save/restore) or because of shared-resource starvation (VZ teardown + WAL checkpoint + virtiofs drain all competing for main-thread and I/O bandwidth) -- park a `tokio::sync::Mutex<()>` on the service's shared state struct and acquire it at the top of the handler for the whole duration of the critical section. `Mutex<()>` isn't a weird construction: the unit value is the lock-token, the type signals "pure serialization, no protected payload". `Semaphore::new(1)` is equivalent -- pick one and stay consistent.

Current instances in `crates/capsem-service/src/main.rs`:

- **`save_restore_lock`**: serializes Apple VZ `saveMachineStateToURL` / `restoreMachineStateFromURL` across sibling VMs. Concurrent save/restore corrupts the VirtioFS ring state on the unlucky VM, surfaces as ext4-on-loop0 I/O errors after resume. Held through `handle_suspend` (IPC + child-exit wait) and `handle_resume` (spawn + `wait_for_vm_ready`). See `docs/src/content/docs/gotchas/concurrent-suspend-resume.md`.

- **`shutdown_lock`**: serializes VM teardown across `handle_delete` / `handle_stop` / `handle_purge` / `handle_run`. Without it, N concurrent deletes under load starve each other of the bandwidth each `capsem-process` needs to exit cleanly within the 1s fast-path budget; past the budget the service SIGKILLs mid-checkpoint and leaves a non-empty `session.db-wal`. Held through `shutdown_vm_process` for the whole `SIGTERM` + `wait_for_process_exit` window.

When to reach for this pattern:

- Symptom is "works solo, fails under concurrency on the same host."
- Root cause is a *per-host* resource, not per-VM: Apple VZ main thread, virtiofsd, DbWriter checkpoint, APFS fsync.
- Production runs exactly one service per host per user, so an in-process tokio mutex is enough -- no need for a file-lock or distributed primitive.

When NOT to reach for it:

- If the contention is per-VM (two handlers acting on the same VM), protect the VM entry in `instances: Mutex<HashMap<...>>` instead.
- If the "contention" is really a durability race (writer thread hasn't flushed), the right fix is usually the signal-handler explicit-cleanup pattern below, not another serialization lock.

### Signal-driven explicit cleanup for background-thread owners

Any long-running Rust process that owns background threads (SQLite writer, notify PollWatcher, MCP aggregator subprocess, vsock relay) and runs under a bounded SIGTERM-to-SIGKILL budget must NOT rely on `Drop` + tokio-runtime-drop ordering to finish cleanup. On SIGTERM, hand owned resources to the signal handler and drain them synchronously BEFORE letting the main run loop return.

Symptom when this is missing: under concurrent teardowns on one host, the service SIGKILLs a child mid-checkpoint or mid-flush. Visible as `session.db-wal` left non-empty, missing `fs_events` rows, dangling aggregator subprocesses. Works solo, fails under `-n 4`.

Concrete primitives in this tree:

- **`DbWriter::shutdown_blocking(&self)`** — takes the stored mpsc sender, joins the writer thread, runs the final `PRAGMA wal_checkpoint(TRUNCATE)`. Arc-safe: other `Arc<DbWriter>` clones remain valid but their writes become no-ops. Idempotent. Drop delegates to it.
- **`FsMonitor::shutdown_and_join(&self)`** — sends on the shutdown channel so the event loop runs its final flush, then joins the thread. Must run BEFORE DbWriter shutdown, because fs_events fan into DbWriter.
- **`CAPSEM_TEST_SLOW_CHECKPOINT_MS`** — test-only env var in `writer_loop` that inserts a sleep before the final checkpoint. Use in tests that need to distinguish explicit cleanup from implicit runtime-drop ordering.

Canonical wiring in `crates/capsem-process/src/main.rs`:

```rust
struct Shutdown {
    db: Option<Arc<DbWriter>>,
    fs_monitor: Option<FsMonitor>,
}

impl Shutdown {
    fn drain_blocking(&mut self) {
        // fs_events fan into DbWriter -- flush fs_monitor first.
        if let Some(m) = self.fs_monitor.take() { m.shutdown_and_join(); }
        if let Some(db) = self.db.take() { db.shutdown_blocking(); }
    }
}

// Populate as owners are constructed:
shutdown.lock().await.db = Some(Arc::clone(&db));
shutdown.lock().await.fs_monitor = Some(monitor);

// Signal handler drains through spawn_blocking, then stops the run loop:
rt.spawn(async move {
    /* wait on SIGTERM/SIGINT */
    let mut owned = std::mem::take(&mut *shutdown.lock().await);
    let _ = tokio::task::spawn_blocking(move || owned.drain_blocking()).await;
    unsafe { core_foundation_sys::runloop::CFRunLoopStop(...); }
});
```

Key properties:

1. **Deterministic order.** The drain order is explicit (fs_monitor -> db), not "whatever reverse-declaration-order Drop happens to give us after tokio aborts tasks."
2. **Synchronous join.** The handler waits for each background thread to finish. No "hope the task finishes before the runtime drops."
3. **Run loop stops last.** `CFRunLoopStop` (macOS) fires only after drain returns. Main returns afterwards; the remaining tokio-runtime drop is now a no-op fast path because the heavy work already completed.
4. **Arc-safe shutdown APIs.** `shutdown_blocking(&self)` works through a shared `Arc<DbWriter>` — callers don't have to chase down every clone. Use `std::sync::Mutex<Option<Sender>>` internally; the hot-path `write()` clones the sender under the lock and releases it before `.await`.

When to reach for this pattern:

- The process has `std::thread::spawn` or `tokio::task::spawn_blocking` workers that run durability-critical work on shutdown (WAL checkpoint, queue flush, child-process wait).
- A parent sends SIGTERM then SIGKILLs after a short, fixed budget.
- Today's cleanup relies on Drop running inside tokio task abort — i.e., you can't draw a line between "cleanup finished" and "run loop exited."

Call out when NOT to use it:

- One-shot CLIs that exit on natural task completion (no run loop, no signal window).
- Workers whose only side effects are in-memory (no durability to lose).

When adding a new long-running process or a new background-thread owner, wire it through `Shutdown` from day one. Don't ship a new binary that "should be fine because Drop will run" — under load, Drop won't run in time.

## Logging

- `tracing` crate with `FmtSpan::CLOSE` for timing spans
- `RUST_LOG=capsem=debug` for full boot timing breakdown
- `RUST_LOG=capsem=info` for top-level only
- Use structured fields: `tracing::info!(domain = %domain, status = %code, "request completed")`

## Lessons learned

1. **Content-Encoding**: Always handle response decompression generically. Gzip compressed SSE responses caused NULL telemetry because the parser got binary garbage. Never strip Accept-Encoding as a workaround.

2. **Platform type widths**: `as _` is your friend for cross-platform libc calls. Explicit casts (`as c_ulong`) will fail on the other platform.

3. **Debouncer timing**: If a VM shuts down before debounced events flush, telemetry is lost. Add `sleep 1` in test commands, or use explicit flush on shutdown.

4. **VirtioFS whiteouts**: Apple VZ's VirtioFS doesn't support `mknod`, so overlayfs can't use it directly as upper. The ext4 loopback workaround provides full POSIX.

5. **setsid for controlling terminal**: Without `setsid`, the PTY has no foreground process group and Ctrl-C (SIGINT) is not delivered. `capsem-init` uses `setsid` to fix this.

6. **serde_json::Value on LLM hot path**: Three ai_traffic struct fields (`ResponseInfo.output`, `FunctionResponse.response`, `FunctionCall.args`) used `serde_json::Value` for large payloads that were only stringified. This forced full DOM allocation on every streaming request. Fixed by removing unused fields and switching to `Box<serde_json::value::RawValue>`.

7. **Prefer syscalls over subprocesses**: `std::process::Command` costs 5-30ms per spawn (fork/exec). If a syscall does the same thing, use it. Example: `cp -c -R` for APFS clonefile was 20-30ms; direct `libc::clonefile()` is <1ms. On Linux, `ReflinkSnapshot` already uses `FICLONE` ioctl directly -- no subprocess. Always check if the OS provides a syscall before reaching for `Command`.

7. **Blocking I/O in MCP gateway**: All 7 snapshot file tool handlers ran blocking I/O (clonefile subprocess, walkdir, blake3) directly on tokio worker threads while holding a `tokio::sync::Mutex`. The auto-snapshot timer did the same. This caused snapshot creation to hang from the model's perspective. Fixed by wrapping in `spawn_blocking` everywhere.

7. **Single-file CoW**: Added `clone_file()` helper that uses APFS clonefile on macOS and FICLONE on Linux for instant CoW copies. Used in snapshot compact (host-to-host). **Not safe for revert** (snapshot-to-VirtioFS-workspace) because APFS clonefile is metadata-only and VirtioFS may serve stale data to the guest. Revert must use `std::fs::copy` (byte copy) so the guest sees the new content immediately.

8. **Platform-gate all macOS-only APIs**: Any code using macOS-only symbols (`libc::clonefile`, Apple framework bindings, etc.) must be wrapped in `#[cfg(target_os = "macos")]` -- both the struct/impl and the tests. The Linux app build (Tauri deb/AppImage) compiles the full workspace; ungated macOS symbols cause `cannot find function` errors on Linux CI. This burned v0.14.7: `ApfsSnapshot` used `libc::clonefile` without a cfg gate. Rule: when adding platform-specific code, gate the definition, the impl, and the tests.

9. **Readiness gates must reflect actual state**: `handle_ipc_connection` responded to Ping with Pong the moment the UDS socket existed -- before vsock connections, boot handshake, or command handler spawn. `wait_for_vm_ready` treated Pong as "ready", so exec commands were sent to a process that couldn't handle them yet, blocking silently in a channel until `setup_vsock` finished. Tests masked this with `wait_exec_ready()` client-side retry loops, creating a double-wait: 30 client retries x 30s server wait each. Fix: `Arc<AtomicBool>` (`vm_ready`) gated by `setup_vsock` after BootReady; IPC handler only sends Pong when the flag is set. One wait, one place -- the server waits; the client calls once. When adding any new IPC readiness check, never respond "ready" based on socket existence alone; check actual process state via a shared flag or state enum.

10. **VirtioFS and FSEvents**: Apple VZ VirtioFS guest writes bypass macOS FSEvents (the kernel's file notification subsystem). If you need to monitor a host directory that is mounted into a guest via VirtioFS, `notify::RecommendedWatcher` will silently drop guest-originated events. You MUST use `notify::poll::PollWatcher` to detect guest file modifications reliably.

11. **Process sandbox: env_clear() on child spawn**: When spawning a child process (e.g., capsem-process from service), always call `env_clear()` then re-add only the minimal env vars needed (`HOME`, `PATH`, `USER`, `TMPDIR`, `RUST_LOG`). The service's shell environment may contain API keys, tokens, or secrets that the child process has no business seeing. The guest's `--env` args are a separate injection path and are already validated.

12. **UDS socket permissions must be 0600**: After `UnixListener::bind()`, immediately `set_permissions(..., 0o600)`. The default umask leaves sockets world-accessible, meaning any local user can connect to a VM's IPC or terminal WebSocket with no auth. The gateway token file already does this; per-VM sockets must match.

13. **Never process::exit() on guest-controlled I/O**: A guest can close a vsock fd at any time. If the host handler calls `process::exit(1)` on read error, the guest has an unconditional DoS. Use `break` to exit the read loop and let the process shut down through normal channels.

14. **File permissions for sensitive logs**: `serial.log` contains raw terminal output and may include secrets typed by the user. Create with explicit `mode(0o600)` via `OpenOptionsExt`, and enforce permissions even if the file already exists (re-set with `set_permissions`).

15. **VirtioFS share boundary -- only guest/ subtree**: The VirtioFS share must point at `session_dir/guest/`, not `session_dir` itself. Host-only files (`session.db`, `serial.log`, `auto_snapshots/`, `checkpoint.vzsave`) must stay outside the share. When adding new host-side files to `session_dir`, they are automatically outside the guest boundary. When adding new guest-visible content, put it under `guest/`. Compat symlinks (`session_dir/{system,workspace} -> guest/{system,workspace}`) let existing host code reference the old paths. Use `capsem_core::guest_share_dir(session_dir)` to get the share root.

16. **Use `capsem_core::poll::poll_until` for all async polling**: All "wait until ready" patterns must use the shared `poll_until` utility in `capsem-core/src/poll.rs`. It provides deadline-based timeout, exponential backoff, and structured tracing (attempt count, elapsed time, label). Never write ad-hoc `for _ in 0..N { sleep(X) }` or `while now < deadline { sleep(fixed) }` loops -- they lack logging, use fixed intervals instead of backoff, and hardcode timeouts. For sync code (guest agent), `vsock_connect_retry` in `vsock_io.rs` has the same pattern with `eprintln` logging. Every retry loop must have a total deadline.

17. **DRY wait patterns -- one wait, one place**: When a server endpoint already waits for a subprocess to become ready (e.g., `wait_for_vm_ready` in `handle_exec`), clients must not add their own retry loop on top. The test helper `wait_exec_ready` previously polled 30 times with 1s sleep, and each poll triggered a 30s server-side wait -- a 30x30s pathological cascade. After fixing the readiness gate (lesson 9), the client calls exec once with adequate HTTP timeout and the server handles the wait. Apply this DRY principle to any client/server readiness pattern: decide which layer owns the wait and make others pass through.

18. **Companions must not outlive their parent -- `kill_on_drop` is not enough**: `tokio::process::Command::kill_on_drop(true)` only fires when the parent's `Child` handle is dropped on graceful shutdown. Under SIGKILL/OOM/test-harness timeout/ pytest-xdist worker death, Drop never runs and companion processes (gateway, tray) get re-parented to PID 1 and survive forever. Every `just test -n 4` run leaked a fresh batch of orphans; accumulated orphans caused VM-ready poll spins, UDS-port collisions, and the suspend/resume regression. Defense in depth is mandatory for any spawned companion process, enforced on the COMPANION side so the parent can't get it wrong:
    - Pass `--parent-pid <spawner_pid>` when spawning.
    - Companion calls `capsem_guard::install(parent_pid, lock_path)?` at startup:
      - Refuses to run if parent PID is missing, dead, or not our actual `getppid()` (`parent_is_expected`). Exit 0 — standalone launches become silent no-ops.
      - Acquires an `flock(2)` singleton at `lock_path` (O_CLOEXEC opened atomically; process-local registry covers the brief fork-to-exec window where the flock fd can be inherited). Second instance exits 0.
      - Spawns a 500ms-interval watcher thread that calls `std::process::exit(0)` the moment `getppid()` no longer equals the declared parent PID. `getppid()` is immune to zombie state and flips to 1 on re-parenting, which is the reliable signal across SIGKILL, SIGSEGV, and OOM.
    - Lock paths: tray is SYSTEM-WIDE (`$HOME/.capsem/run/tray.lock`) because the macOS menu bar is a shared global resource; gateway is per-run_dir because each test's gateway bridges a distinct UDS. Regression tests in `tests/capsem-service/test_companion_lifecycle.py` cover: refuse-standalone (no parent / wrong parent), singleton (double spawn, 20-way hammer), and die-with-parent (SIGKILL the parent, companion exits within 5s). When adding any new companion process, wire it through `capsem-guard` — don't invent a new pattern.

19. **Retry loops must classify errors, not time-bound a blanket wait**: When waiting for a resource to come up, the retry closure must distinguish *retryable* errors from *permanent* ones, and the classification depends on the caller's context, not the error itself. Identical `NotFound` / `ConnectionRefused` errors mean "service is down, give up" on an initial probe but "socket not bound yet, keep waiting" one call later in the post-launch retry. Pattern:
    - **Use `capsem_core::poll::poll_until`, not a hand-rolled backoff loop.** The poll primitive already gives you deadline, exponential backoff, per-attempt logging (label + elapsed + attempts), and a typed `TimedOut` error. Every new retry site that reinvents these is a future bug -- the `capsem doctor` "Service manager started capsem but socket not ready" bug existed only because `UdsClient::connect_with_timeout` hand-rolled its own loop and fast-failed on `ENOENT` before the just-started service had bound its socket.
    - **Inside the `poll_until` closure:** return `None` on retryable errors (`poll_until` keeps polling), return `Some(Err(...))` on permanent errors (`poll_until` exits immediately).
    - **Thread a small enum, not a `patient: bool`**, so every call site documents intent: `ConnectMode::FailFast` vs `ConnectMode::AwaitStartup`, `ProbeMode::Expected` vs `ProbeMode::MustBeRunning`. `crates/capsem/src/client.rs::UdsClient::connect_with_timeout` is the canonical example in the tree.
    - **Don't `.map_err(|_| anyhow!(...))` on the timeout branch.** You erase the inner cause. Chain with `Context` so the root error lives in the error chain and `{err:#}` prints both the summary and the underlying io::Error kind.

## Async reference

Read `references/rust-async-patterns.md` for comprehensive tokio patterns (tasks, channels, streams, error handling). From the community (6.4K installs).
