# Crate and Privilege Model

Read this reference before moving responsibilities between crates, changing
capsem-process permissions or environment, changing session/socket/share
boundaries, or changing MITM CA-key handling.

## Crate architecture

- **`capsem-core`**: all shared logic (VM, network, policy, telemetry, config). This is where business logic lives.
- **`capsem-service`**: daemon process. Axum HTTP server over UDS, spawns/manages capsem-process children, routes API calls via IPC.
- **`capsem-process`**: per-VM process. Boots VM via capsem-core, bridges vsock, manages structured jobs (exec, file I/O) with a job store + oneshot channels.
- **`capsem`**: CLI client. HTTP over UDS to service, direct UDS to process for shell.
- **`capsem-mcp`**: MCP server (stdio). Uses `rmcp` crate. Bridges AI agent tool calls to service HTTP API.
- **`capsem-gateway`**: TCP-to-UDS HTTP reverse proxy. Axum server on port 19222, Bearer token auth, CORS. Provides an explicit route table, `/status` (cached), and `/terminal/{id}` (WebSocket relay). Unknown routes return 404; frontend and tray connect through this.
- **`capsem-app`**: thin Tauri webview shell. Points at gateway (`http://127.0.0.1:19222`). No capsem-core dependency. 2 IPC commands: `open_url`, `check_for_app_update`.
- **`capsem-agent`**: guest binaries crate. Contains four binaries cross-compiled for aarch64/x86_64-linux-musl: `capsem-pty-agent` (PTY bridge + control + exec + file I/O + shutdown), `capsem-sysutil` (guest suspend helper; in-VM shutdown disabled), `capsem-net-proxy` (HTTPS -> MITM), `capsem-mcp-server` (guest MCP relay).
- **`capsem-logger`**: session DB schema, queries, async writer.
- **`capsem-proto`**: shared protocol types. `ipc.rs` (ServiceToProcess/ProcessToService), `lib.rs` (HostToGuest/GuestToHost).

## Process privilege model

capsem-process is a **low-privilege** per-VM process. Security invariants:

1. **Minimal environment**: service uses `env_clear()` before spawn, then passes only `HOME`, `PATH`, `USER`, `TMPDIR`, `RUST_LOG`. API keys and tokens from the user's shell never reach the process.
2. **Socket permissions 0600**: IPC (`{id}.sock`) and terminal WS (`{id}-ws.sock`) sockets are chmod 0600 after bind. Only the owning user can connect.
3. **Session directory 0700**: created by the service via `create_virtiofs_session`. Contains workspace/, system/, serial.log (0600), session.db.
4. **No guest-triggered process exit**: control channel read errors cause `break` (loop exit), not `process::exit()`. Guest cannot DoS the host process.
5. **Gateway auth layer**: external access goes through capsem-gateway (Bearer token, rate limiting, localhost CORS). Per-VM sockets are not exposed to the network.
6. **Rootfs read-only**: profile rootfs asset mounted read-only. Guest binaries deployed chmod 555.
7. **Guest binary security**: all injected binaries are read-only. Guest cannot modify its own agent.
8. **VirtioFS boundary**: only `session_dir/guest/` is shared via VirtioFS (contains `system/` and `workspace/`). Host-only files (`session.db`, `serial.log`, `auto_snapshots/`, `checkpoint.vzsave`) are outside the share. Compat symlinks at `session_dir/{system,workspace}` point into `guest/` so existing code paths work unchanged.

### What capsem-process CAN access
- Its own session_dir (read-write)
- Assets dir (read-only: kernel, initrd, rootfs)
- Its own UDS sockets
- Apple VZ framework (requires `com.apple.security.virtualization` entitlement)

### What capsem-process CANNOT access
- Other VMs' session dirs (0700, different path)
- Other VMs' UDS sockets (0600)
- The service's UDS socket (filesystem permission only)
- The persistent registry or other service state
- The user's environment variables (cleared at spawn)

### MITM CA key transparency
The MITM proxy CA private key (`config/capsem-ca.key`) is committed to the repo and embedded at compile time. This is intentional -- capsem's network interception exists for user visibility into what AI agents do, not for secrecy. The CA is only trusted inside capsem's own air-gapped VMs and has zero trust outside them. A public key lets anyone verify there is no hidden interception. Per-installation key generation would reduce transparency.
