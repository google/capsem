# Crate and Privilege Model

Read this reference before moving responsibilities between crates, changing
capsem-process permissions or environment, changing session/socket/share
boundaries, or changing MITM CA-key handling.

## Crate architecture

Reusable code belongs to the lowest-dependency crate that owns its domain.
Sharing alone is not a reason to put code in `capsem-core`.

- **`capsem-foundation`**: dependency-light host primitives: paths, UDS HTTP,
  polling, telemetry/log setup, and IPC handshakes.
- **`capsem-assets`**: asset manifests, compatibility, download, resolution,
  and verification.
- **`capsem-config`**: config types, parsing, validation, resolution, and
  provider/MCP identity.
- **`capsem-credentials`**: credential provider contracts and durable store.
- **`capsem-proto`**: shared host/guest and service/process wire contracts.
- **`capsem-core`**: VM, hypervisor, security-engine, host-network, MCP runtime,
  and session/image domain logic.
- **`capsem-logger`**: session DB schema, queries, storage, and async writer.
- **`capsem-guard`**: parent-watch and singleton-flock lifecycle primitives.
- **`capsem-service`**: daemon HTTP/UDS API and VM-process orchestration.
- **`capsem-process`**: low-privilege per-VM boot, vsock, IPC, and job runtime.
- **`capsem`**: CLI client; HTTP/UDS to the service and direct process UDS for shell.
- **`capsem-tui`**: terminal control UI over the gateway API.
- **`capsem-admin`**: profile/asset/release validation and materialization.
- **`capsem-mcp`**: host MCP server bridging AI-agent tools to the service API.
- **`capsem-mcp-aggregator`**: low-privilege external-MCP subprocess manager.
- **`capsem-mcp-builtin`**: built-in HTTP and file/snapshot MCP tools.
- **`capsem-gateway`**: authenticated TCP-to-UDS HTTP/WebSocket gateway.
- **`capsem-app`**: thin Tauri webview shell pointing at the gateway.
- **`capsem-tray`**: system tray status and quick actions through the gateway.
- **`capsem-agent`**: musl guest PTY, network, DNS, MCP, and sysutil binaries.
- **`capsem-bench`**: host/guest benchmark harness and collectors.
- **`capsem-mock-server`**: hermetic HTTP/TLS/WebSocket test upstream.

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
The MITM proxy CA private key (`crates/capsem-core/resources/ca/capsem-ca.key`) is committed to the repo and embedded at compile time. This is intentional -- capsem's network interception exists for user visibility into what AI agents do, not for secrecy. The CA is only trusted inside capsem's own air-gapped VMs and has zero trust outside them. A public key lets anyone verify there is no hidden interception. Per-installation key generation would reduce transparency.
