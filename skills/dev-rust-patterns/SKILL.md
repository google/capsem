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

## Memory and resource management

- **File handle limits**: VirtioFS caps at 4096 open file handles, returns `EMFILE` beyond that.
- **Read size limits**: VirtioFS clamps reads to 1MB, gather buffers to 2MB.
- **Safe deserialization**: `read_struct` returns `Option<T>` with bounds checks in all builds (not just debug).
- **irqfd for interrupt delivery**: Guest interrupt signaling uses `irqfd` to avoid cross-thread syscall overhead.

## Concurrency patterns

- **RwLock for caches**: Cert authority uses `RwLock<HashMap>` -- many readers, rare writers. Use `read()` first, upgrade to `write()` only on cache miss.
- **Arc for shared state**: VM state, proxy config, and telemetry handles are `Arc`-wrapped for sharing across tasks.
- **Per-connection tasks**: The MITM proxy spawns a new tokio task per connection. Each task owns its TLS state and upstream connection. No shared mutable state between connections.

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

## Async reference

Read `references/rust-async-patterns.md` for comprehensive tokio patterns (tasks, channels, streams, error handling). From the community (6.4K installs).
