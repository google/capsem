---
name: site-architecture
description: Capsem system architecture -- service daemon, per-VM processes, CLI, MCP server, guest agent, vsock, network proxy. Use when you need to understand the system design to write code, review changes, write documentation, or debug cross-component issues. Covers the service architecture, IPC protocols, vsock ports, storage modes, network policy, MITM proxy, and key source files.
---

# Capsem Architecture

## System overview

Capsem sandboxes AI agents in air-gapped Linux VMs on macOS using Apple's Virtualization.framework (with a KVM backend for Linux). It runs as a daemon service (like Docker). The system has these layers:

**Host-side:**
- **capsem-service** (daemon): always-running background service. Axum HTTP server over Unix Domain Socket (`~/.capsem/run/service.sock`). Manages VM lifecycle, routes API calls to per-VM processes.
- **capsem-process** (per-VM): one process per sandbox. Boots the VM, bridges vsock connections (terminal + control), manages structured jobs (exec, file I/O) via a job store.
- **capsem** (CLI): user-facing CLI. **Everything is ephemeral unless asked otherwise.** `capsem shell` (no args) = temp VM + auto-destroy on exit. `capsem create -n <name>` = persistent VM (detached). `capsem create` (no name) = ephemeral VM (detached). `capsem shell <id>` = attach to existing. Talks to capsem-service over UDS HTTP.
- **capsem-mcp** (MCP server): stdio-based MCP server for AI agents (Claude Code, Gemini CLI). Bridges MCP tool calls to capsem-service HTTP API.
- **capsem-gateway** (HTTP gateway): TCP-to-UDS reverse proxy (default port 19222). Bearer token auth, CORS, 10MB body limit. Provides `/status` (cached 1s), `/terminal/{id}` (WebSocket relay to per-VM UDS), and transparent fallback proxy to capsem-service. The frontend and tray app connect through the gateway. Writes runtime files to `~/.capsem/run/` (gateway.token, gateway.port, gateway.pid).
- **capsem-app** (Tauri GUI): optional GUI shell with xterm.js frontend. All VM operations go through capsem-service (no direct VM boot).
- **capsem-tray** (system tray): standalone menu bar process. Polls the gateway for VM status, shows running/stopped counts, and provides quick actions (open dashboard, quit). Runs in its own process, isolated from the service.

**Guest-side:**
- **capsem-init** (`capsem-init`): PID 1, sets up air-gapped networking, mounts filesystems, deploys guest binaries, launches daemons, writes boot timing JSONL
- **capsem-pty-agent** (`capsem-pty-agent`): main guest agent -- PTY bridge, control channel, exec, file I/O, shutdown handler (see "Guest agent architecture" below)
- **capsem-sysutil** (`capsem-sysutil`): multi-call binary for guest lifecycle commands (shutdown, halt, poweroff, reboot, suspend). Opens its own vsock:5004 connection independently of the agent, so shutdown works even if the agent is hung. Symlinked by capsem-init to `/sbin/shutdown`, `/sbin/halt`, `/sbin/poweroff`, `/sbin/reboot`, `/usr/local/bin/suspend`.
- **capsem-net-proxy** (`capsem-net-proxy`): redirects HTTPS traffic to host MITM proxy via vsock
- **capsem-mcp-server** (`capsem-mcp-server`): in-guest MCP gateway, routes tool calls to external MCP servers via vsock

## Service architecture

**All VM operations go through a single path.** There is no direct VM boot -- every entry point routes through capsem-service to capsem-process.

```
AI Agent  -> capsem-mcp (stdio)  -> HTTP/UDS -> capsem-service
User      -> capsem CLI          -> HTTP/UDS -> capsem-service
Frontend  -> capsem-gateway (TCP)-> HTTP/UDS -> capsem-service
Tray app  -> capsem-gateway (TCP)-> HTTP/UDS -> capsem-service
                                                     |
                                        capsem-process (per-VM, UDS IPC)
                                                     |
                                         +-----------+-----------+
                                         |           |           |
                                    vsock:5000  vsock:5001  vsock:5005
                                    (control)  (terminal)  (exec output)
                                         |           |           |
                                         +-----guest agent------+
```

**Entry points for exec:**
- `capsem exec <id> "cmd"` -> service HTTP `/exec/{id}` -> process IPC -> vsock
- `capsem run "cmd"` -> service HTTP `/run` -> provision + exec + destroy
- MCP `capsem_exec` / `capsem_run` -> service HTTP -> same path

**Entry point for interactive shell:**
- `capsem shell [id]` -> UDS IPC directly to capsem-process -> `StartTerminalStream` -> vsock:5001

### IPC protocols

| Layer | Protocol | Socket |
|-------|----------|--------|
| Frontend/Tray -> gateway | HTTP/1.1 over TCP | `127.0.0.1:19222` (Bearer token auth) |
| Gateway -> service | HTTP/1.1 over UDS | `~/.capsem/run/service.sock` |
| CLI/MCP -> service | HTTP/1.1 over UDS | `~/.capsem/run/service.sock` |
| Service -> process | MessagePack over UDS | `~/.capsem/run/instances/{id}.sock` |
| Process -> guest agent | Binary frames over vsock | ports 5000 (control), 5001 (terminal), 5004 (lifecycle), 5005 (exec) |

### Service HTTP API

| Method | Path | Purpose |
|--------|------|---------|
| POST | `/provision` | Create a new sandbox VM (set `persistent: true` for named VMs) |
| GET | `/list` | List all sandboxes (running + stopped persistent) |
| GET | `/info/{id}` | Sandbox details (config, status, persistent) |
| POST | `/exec/{id}` | Execute command, return stdout/stderr/exit_code |
| POST | `/run` | One-shot: provision temp VM, exec command, destroy, return output |
| POST | `/stop/{id}` | Stop VM (persistent: preserve state; ephemeral: destroy) |
| POST | `/resume/{name}` | Resume a stopped persistent VM |
| POST | `/persist/{id}` | Convert running ephemeral VM to persistent |
| POST | `/purge` | Kill all temp VMs (set `all: true` to include persistent) |
| POST | `/write_file/{id}` | Write file to guest |
| GET | `/read_file/{id}?path=...` | Read file from guest |
| GET | `/logs/{id}` | Serial/boot logs |
| POST | `/inspect/{id}` | Raw SQL query against session.db |
| DELETE | `/delete/{id}` | Destroy VM and wipe all state |
| POST | `/fork/{id}` | Fork a VM into a reusable image |
| GET | `/images` | List all user images |
| GET | `/images/{name}` | Inspect a specific image |
| DELETE | `/images/{name}` | Delete an image |

### MCP tools (capsem-mcp)

21 tools: `capsem_create` (env + image params), `capsem_list`, `capsem_info`, `capsem_exec` (timeout param), `capsem_run`, `capsem_stop`, `capsem_resume`, `capsem_persist`, `capsem_purge`, `capsem_read_file`, `capsem_write_file`, `capsem_vm_logs` (grep + tail), `capsem_service_logs` (grep + tail), `capsem_inspect_schema`, `capsem_inspect`, `capsem_delete`, `capsem_version`, `capsem_fork`, `capsem_image_list`, `capsem_image_inspect`, `capsem_image_delete`.

## Host-guest communication

All host-guest communication flows through capsem-process via vsock. There is no direct vsock access from any other host binary.

```
Interactive shell:  capsem-process -> vsock:5001 <-> Guest PTY (bash)
Exec command:       capsem-process -> vsock:5000 (Exec cmd) -> Guest agent
                    capsem-process <- vsock:5005 (stdout)    <- Guest child process
                    capsem-process <- vsock:5000 (ExecDone)  <- Guest agent
File I/O:           capsem-process -> vsock:5000 (FileWrite/FileRead) <-> Guest agent
```

Terminal I/O flows through vsock port 5001 (raw PTY bytes). Exec output flows on a dedicated port 5005 connection -- completely separated from the interactive terminal. File I/O uses port 5000 (control channel).

Serial console stays active for kernel boot logs. Terminal I/O switches to vsock once the guest agent sends `Ready`.

### Vsock ports

| Port | Purpose |
|------|---------|
| 5000 | Control messages (resize, heartbeat, exec commands, file I/O) |
| 5001 | Terminal data (PTY I/O) |
| 5002 | MITM proxy (HTTPS connections) |
| 5003 | MCP gateway (tool routing, NDJSON passthrough) |
| 5004 | Lifecycle commands (shutdown/suspend, capsem-sysutil) |
| 5005 | Exec output (direct child process stdout, on demand) |

## Guest agent architecture

All guest binaries live in `crates/capsem-agent/` and are cross-compiled for `aarch64-unknown-linux-musl` (and `x86_64-unknown-linux-musl`). Deployed chmod 555 (read-only) into the initrd at `/run/`.

### capsem-pty-agent (main agent)

Single-threaded, sync Rust binary (no tokio). Launched by capsem-init after filesystems are mounted.

**Boot sequence:**
1. Connect to host on vsock:5001 (terminal) and vsock:5000 (control)
2. Send `GuestToHost::Ready` with agent version
3. Boot handshake: receive `BootConfig` (clock sync), then `SetEnv`/`FileWrite` messages, then `BootConfigDone`
4. Apply env vars, write files, set hostname from `CAPSEM_VM_NAME`
5. Open PTY pair, fork bash on the slave side
6. Send `GuestToHost::BootReady` + `BootTiming` (parsed from capsem-init's JSONL)
7. Enter bridge loop

**Runtime -- two loops running concurrently:**
- **bridge_loop** (main thread): polls master PTY, forwards output to vsock:5001. Spawns a dedicated thread for the reverse direction (vsock -> PTY). Pure bidirectional byte bridge with no scanning or filtering.
- **control_loop** (background thread): reads vsock:5000, handles `Resize` (set winsize + SIGWINCH), `Ping`/`Pong` heartbeat, `Exec` (spawns background thread for direct child process), `FileWrite`/`FileRead`/`FileDelete`, and `Shutdown`.

**Exec mechanism:** spawns `bash -c '<cmd> 2>&1'` as a direct child process (not via PTY). Connects to host on vsock:5005, sends `ExecStarted { id }` handshake, then streams child stdout to the exec port. Exit code comes from `waitpid`, sent as `ExecDone { id, exit_code }` on vsock:5000. Runs in a background thread so control_loop stays responsive to heartbeats during long commands.

**Shutdown handler:** `sync()` -> `SIGTERM` bash -> wait `SHUTDOWN_GRACE_SECS` (defined in `capsem-proto`) -> `SIGKILL` (interactive bash ignores SIGTERM) -> break. The bridge loop cleanup then sends SIGHUP + waitpid to reap the child.

### capsem-sysutil (lifecycle multi-call binary)

Busybox-pattern binary dispatching on `argv[0]`. Symlinked by capsem-init:
- `/sbin/shutdown`, `/sbin/halt`, `/sbin/poweroff`, `/sbin/reboot` -> `/run/capsem-sysutil`
- `/usr/local/bin/suspend` -> `/run/capsem-sysutil`

Opens its own vsock:5004 connection (independent of capsem-pty-agent) and sends `GuestToHost::ShutdownRequest` or `SuspendRequest`. Shows a countdown (`SHUTDOWN_GRACE_SECS + 1` seconds) before sending. Rejects reboot requests with an error.

**Shutdown flow (end-to-end):**
```
Guest: shutdown -> capsem-sysutil -> vsock:5004 -> capsem-process
  capsem-process: reads ShutdownRequest -> sends ProcessToService::ShutdownRequested to service
  capsem-process: sends HostToGuest::Shutdown on control channel (vsock:5000)
  capsem-pty-agent: receives Shutdown -> sync + SIGTERM + grace + SIGKILL -> exit
  capsem-process: VM stops, process exits
  capsem-service: child reaper cleans up (ephemeral: destroy session, persistent: preserve)
```

### capsem-net-proxy

Listens on localhost:10443 inside the guest. iptables redirects all port 443 traffic here. Each connection is bridged to host vsock:5002 where the MITM proxy handles TLS termination and policy.

### capsem-mcp-server

In-guest MCP gateway. Listens for MCP tool calls and routes them to external MCP servers via vsock:5003 NDJSON passthrough.

## Storage modes

Selected by kernel cmdline `capsem.storage=virtiofs` (default) or absence (block mode).

**VirtioFS mode** (default):
```
~/.capsem/sessions/{id}/
  system/rootfs.img    # ext4 loopback (2GB sparse) -- overlayfs upper
  workspace/           # VirtioFS files for /root (host-visible)
  auto_snapshots/      # Rolling ring buffer (12 APFS clones, 5min interval)
```

Boot sequence: squashfs -> VirtioFS mount -> loopback ext4 -> overlayfs -> bind-mount workspace.

Why ext4 loopback: Apple VZ's VirtioFS doesn't support `mknod` (whiteout creation), so overlayfs can't use VirtioFS directly as upper.

**Block mode** (legacy): tmpfs overlay + scratch disk. No host file visibility, no snapshots.

**Fork images** (user-created templates):
```
~/.capsem/images/
  image_registry.json       # Image metadata index (JSON)
  {name}/
    system/                  # APFS clone of source VM's rootfs overlay
    workspace/               # APFS clone of workspace files
    session.db               # Telemetry from source VM (checkpointed)
```

## Network architecture

The guest is air-gapped. No real NIC, no real DNS, no direct internet access.

1. `capsem-init` creates a dummy0 NIC with fake DNS (dnsmasq)
2. iptables redirects all port 443 traffic to `capsem-net-proxy` on localhost:10443
3. `capsem-net-proxy` bridges each TCP connection to host vsock port 5002
4. Host MITM proxy terminates TLS using per-domain minted certs (signed by static Capsem CA)
5. Host inspects HTTP request, applies domain + HTTP policy, forwards to real upstream
6. Full telemetry recorded to session DB (domain, method, path, status, headers, body preview)

### Network policy

- User config: `~/.capsem/user.toml` -- domain allow/block lists + HTTP rules
- Corp config: `/etc/capsem/corp.toml` -- enterprise lockdown (MDM-distributed)
- Merge: corp overrides user entirely per field; unspecified fields fall through
- HTTP rules: `[[network.rules]]` with method+path matching per domain

### MITM CA

- Static CA: `config/capsem-ca.key` + `config/capsem-ca.crt` (ECDSA P-256)
- Baked into rootfs via `update-ca-certificates` + certifi patch
- Guest trusts it via system store + env vars (`REQUESTS_CA_BUNDLE`, `NODE_EXTRA_CA_CERTS`, `SSL_CERT_FILE`)

## Ephemeral VM model (invariants)

**VirtioFS mode**: fresh workspace + sparse rootfs.img per session. Host creates empty dirs, guest formats on first boot.

**Block mode**: `mke2fs` runs unconditionally at boot. Overlay upper is always tmpfs.

**Everything is ephemeral unless asked otherwise.** VMs are temporary by default. Named VMs (`capsem create -n <name>`) are persistent -- their workspace and rootfs overlay survive stops and can be resumed. Persistent VM data lives in `~/.capsem/run/persistent/`. Never make the overlay upper layer persistent for ephemeral VMs. To add packages: edit guest config and `just build-assets`.

**Fork images** extend the ephemeral model with reusable templates. `capsem fork <vm> <image-name>` snapshots a VM (running or stopped) via APFS clonefile. `capsem create --image <name>` boots from the template. Images have flat genealogy: each depends only on a base squashfs version, never on other images. Deleting any image is always safe; asset cleanup protects referenced squashfs versions.

## Key source files

Read `references/key-files.md` for the full annotated source map.

## Tauri v2 reference

Read `references/tauri-v2.md` for Tauri IPC patterns, commands, capabilities, and permissions.

## Crate architecture

- **`capsem-core`**: all shared logic (VM, network, policy, telemetry, config). This is where business logic lives.
- **`capsem-service`**: daemon process. Axum HTTP server over UDS, spawns/manages capsem-process children, routes API calls via IPC.
- **`capsem-process`**: per-VM process. Boots VM via capsem-core, bridges vsock, manages structured jobs (exec, file I/O) with a job store + oneshot channels.
- **`capsem`**: CLI client. HTTP over UDS to service, direct UDS to process for shell.
- **`capsem-mcp`**: MCP server (stdio). Uses `rmcp` crate. Bridges AI agent tool calls to service HTTP API.
- **`capsem-gateway`**: TCP-to-UDS HTTP reverse proxy. Axum server on port 19222, Bearer token auth, CORS. Provides `/status` (cached), `/terminal/{id}` (WebSocket relay), and transparent fallback to service. Frontend and tray connect through this.
- **`capsem-app`**: optional Tauri GUI shell. Wires IPC commands to core.
- **`capsem-agent`**: guest binaries crate. Contains four binaries cross-compiled for aarch64/x86_64-linux-musl: `capsem-pty-agent` (PTY bridge + control + exec + file I/O + shutdown), `capsem-sysutil` (lifecycle multi-call: shutdown/halt/poweroff/reboot/suspend), `capsem-net-proxy` (HTTPS -> MITM), `capsem-mcp-server` (MCP gateway).
- **`capsem-logger`**: session DB schema, queries, async writer.
- **`capsem-proto`**: shared protocol types. `ipc.rs` (ServiceToProcess/ProcessToService), `lib.rs` (HostToGuest/GuestToHost).

## Process privilege model

capsem-process is a **low-privilege** per-VM process. Security invariants:

1. **Minimal environment**: service uses `env_clear()` before spawn, then passes only `HOME`, `PATH`, `USER`, `TMPDIR`, `RUST_LOG`. API keys and tokens from the user's shell never reach the process.
2. **Socket permissions 0600**: IPC (`{id}.sock`) and terminal WS (`{id}-ws.sock`) sockets are chmod 0600 after bind. Only the owning user can connect.
3. **Session directory 0700**: created by the service via `create_virtiofs_session`. Contains workspace/, system/, serial.log (0600), session.db.
4. **No guest-triggered process exit**: control channel read errors cause `break` (loop exit), not `process::exit()`. Guest cannot DoS the host process.
5. **Gateway auth layer**: external access goes through capsem-gateway (Bearer token, rate limiting, localhost CORS). Per-VM sockets are not exposed to the network.
6. **Rootfs read-only**: squashfs mounted read-only by Apple VZ. Guest binaries deployed chmod 555.
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
