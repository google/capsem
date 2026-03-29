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

## Async reference

Read `references/rust-async-patterns.md` for comprehensive tokio patterns (tasks, channels, streams, error handling). From the community (6.4K installs).
