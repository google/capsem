# Gemini CLI Directives for Capsem

This file contains foundational mandates and development guidelines for Capsem. When contributing to or modifying this codebase, you MUST adhere to the following principles. They take precedence over general defaults.

## 1. Concurrency and Async I/O (Tokio & Axum)
- **Never Block the Async Executor:** Capsem heavily utilizes `tokio` for async runtimes (e.g., in the Tauri backend and Axum gateway). You must NEVER perform synchronous, long-running operations (like heavy CPU-bound tasks or blocking disk I/O, including SQLite database writes) directly inside an `async` function.
- **Use `spawn_blocking`:** Always wrap synchronous operations (e.g., `rusqlite` execution, large file reads/writes) inside `tokio::task::spawn_blocking` to offload them to a dedicated thread pool. Failure to do so will stall the worker thread, exhaust the thread pool, and freeze the application or gateway.

## 2. Preventing Deadlocks in Bidirectional I/O
- **Decouple I/O Streams:** When bridging two blocking file descriptors bidirectionally (e.g., a TCP socket to a vsock in `net_proxy.rs`, or a master PTY to a vsock in `capsem-pty-agent`), doing both reads and writes in a single thread using `poll(2)` is strictly prohibited. 
- **Thread per Direction:** If both outgoing buffers fill up simultaneously, a single thread will block on writing and stop reading, creating a mutual lockup. Always spawn a dedicated background thread to handle at least one direction of the bidirectional data flow (e.g., `std::thread::spawn` for `fd_b -> fd_a` while the main thread handles `fd_a -> fd_b`).

## 3. Optimizing Payload Parsing (Serde)
- **Avoid Full DOM Parsing:** The LLM Gateway and other network components handle massive HTTP payloads (megabytes of tool calls, histories, or images). Parsing these entirely into dynamic memory structures (like `serde_json::Value`) is highly inefficient and risks memory exhaustion.
- **Targeted Deserialization:** When extracting specific fields from a large JSON payload, ALWAYS define a targeted struct and use `serde::Deserialize`. Serde performs a structural parse, skipping and discarding unused data without allocating memory for it.

## 4. General Architectural Context
- **Rust Backend:** The core functionality resides in `crates/`. 
  - `capsem-core`: The main VM and networking logic.
  - `capsem-agent`: Guest-side binaries compiled for `aarch64-unknown-linux-musl`.
  - `capsem-app`: The Tauri application wrapper.
  - `capsem-proto`: Shared protocol types.
- **Database:** We use `rusqlite` for per-session SQLite databases (`web.db` for network telemetry, `audit_db` for LLM gateway). Always open in WAL mode for better concurrent performance.
- **Networking:** Virtual machine communication happens over Virtio-Vsock. 
  - Port 5000: Control (Boot, Resize, Exec)
  - Port 5001: PTY / Terminal
  - Port 5002: SNI Proxy (HTTPS filtering)
  - Port 5004: AI Gateway (planned)

Always ensure tests are written for concurrent code (e.g., `UnixStream::pair()` to simulate full-duplex backpressure) to proactively catch deadlocks.

## 5. Key Documentation
Before making significant architectural or security-related changes, you MUST consult the following documents:
- **`README.md`**: High-level overview, build instructions, and project goals.
- **`docs/architecture.md`**: Detailed system architecture, boot sequence, execution logging, and future roadmap.
- **`docs/security.md`**: Core security invariants, threat model, and isolation boundaries.