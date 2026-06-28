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
- **capsem** (CLI): user-facing CLI. Sessions are created from profiles and
  named by the service (`<profile-id>-N` unless the user supplies a name).
  `capsem shell` opens the TUI/session picker, creates or attaches through the
  service, and talks to capsem-service over UDS HTTP. User-facing copy says
  sessions; implementation/debug output may say VM when describing the
  virtualization layer.
- **capsem-mcp** (MCP server): stdio-based MCP server for AI agents (Claude Code, Gemini CLI). Bridges MCP tool calls to capsem-service HTTP API.
- **capsem-gateway** (HTTP gateway): TCP-to-UDS reverse proxy (default port 19222). Bearer token auth, CORS, 10MB body limit. Provides an explicit route table plus `/status` (cached 1s) and `/terminal/{id}` (WebSocket relay to per-VM UDS). Unknown routes return 404; the frontend and tray app connect through the gateway. Writes runtime files to `~/.capsem/run/` (gateway.token, gateway.port, gateway.pid).
- **capsem-app** (Tauri GUI): thin webview shell. Connects to gateway at `http://127.0.0.1:19222`. No VM logic, no capsem-core dependency. Only 2 IPC commands: `open_url` (opens URL in system browser) and `check_for_app_update` (Tauri updater). Bundles `frontend/dist` so the app can render the service-unavailable screen when gateway is unreachable.
- **capsem-tray** (system tray): menu-bar companion process. Polls the gateway for VM status, shows running/stopped counts, and provides quick actions (open dashboard, quit). Non-standalone: refuses to run without `--parent-pid` pointing at a live capsem-service, acquires a system-wide singleton lock at `~/.capsem/run/tray.lock` (only one tray ever in the menu bar), and self-exits within 500ms when its parent dies. Contract enforced by `capsem-guard` on the companion side, not the spawner.
- **capsem-guard** (shared library): parent-watch + singleton primitives used by capsem-tray and capsem-gateway. Provides `watch_parent_or_exit`, `Singleton::try_acquire`, and the umbrella `install(parent_pid, lock_path)`. Guarantees companions die with their parent and can't run standalone or as multiple instances -- closes the orphan-accumulation class of bug that `kill_on_drop(true)` alone cannot cover under SIGKILL/OOM/test-harness termination. See `/dev-rust-patterns` lesson 18.

**Guest-side:**
- **capsem-init** (`capsem-init`): PID 1, sets up air-gapped networking, mounts filesystems, deploys guest binaries, launches daemons, writes boot timing JSONL
- **capsem-pty-agent** (`capsem-pty-agent`): main guest agent -- PTY bridge, control channel, exec, file I/O, shutdown handler (see "Guest agent architecture" below)
- **capsem-sysutil** (`capsem-sysutil`): guest suspend helper. Opens its own vsock:5004 connection independently of the agent, so suspend works even if the agent is hung. Symlinked by capsem-init only to `/usr/local/bin/suspend`; in-VM shutdown commands are disabled.
- **capsem-net-proxy** (`capsem-net-proxy`): redirects HTTPS traffic to host MITM proxy via vsock
- **capsem-mcp-server** (`capsem-mcp-server`): guest MCP stdio-to-framed-vsock relay for tool calls to the host MITM MCP endpoint

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

The service and gateway expose one explicit route table. Unknown routes must
return 404; do not add compatibility aliases or generic gateway forwarding. The full
contract lives in `docs/src/content/docs/architecture/service-api.md`; the
common session routes are:

| Method | Path | Purpose |
|--------|------|---------|
| POST | `/vms/create` | Create a session from a profile |
| GET | `/vms/list` | List sessions and profile/status metadata |
| GET | `/vms/{id}/info` | Session identity, profile, config, and diagnostics |
| GET | `/vms/{id}/status` | Hot in-memory runtime state and counters |
| POST | `/vms/{id}/exec` | Execute command, return stdout/stderr/exit_code |
| POST | `/run` | One-shot create + exec + destroy through the same service path |
| POST | `/vms/{id}/stop` | Stop a running session |
| POST | `/vms/{id}/pause` | Pause/suspend a running session |
| POST | `/vms/{id}/start` | Start a stopped session |
| POST | `/vms/{id}/resume` | Resume a paused/stopped session |
| POST | `/vms/{id}/save` | Save current session state |
| POST | `/vms/{id}/fork` | Fork a session into reusable state |
| DELETE | `/vms/{id}/delete` | Destroy session and wipe state |
| POST | `/purge` | Delete defunct/incompatible service state |
| POST | `/vms/{id}/files/write` | Write file to guest |
| POST | `/vms/{id}/files/read` | Read file from guest |
| GET | `/vms/{id}/files/list` | List guest files |
| GET | `/vms/{id}/files/content` | Download file content |
| POST | `/vms/{id}/files/content` | Upload file content |
| GET | `/vms/{id}/logs` | Serial/boot logs |

### MCP tools (capsem-mcp)

MCP tools include `capsem_create`, `capsem_list`, `capsem_info`, `capsem_exec`,
`capsem_run`, lifecycle tools, file read/write, logs, timeline, triage,
version, fork, and profile MCP tools. Raw SQL inspection tools are not part of
the product surface; telemetry access must use typed routes.

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
| 5002 | MITM proxy and framed guest MCP endpoint |
| 5004 | Lifecycle commands (suspend; deprecated shutdown frames ignored, capsem-sysutil) |
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

### capsem-sysutil (guest suspend helper)

Busybox-pattern binary dispatching on `argv[0]`. Symlinked by capsem-init:
- `/usr/local/bin/suspend` -> `/run/capsem-sysutil`

Opens its own vsock:5004 connection (independent of capsem-pty-agent) and sends `GuestToHost::SuspendRequest`. Shows a countdown (`SHUTDOWN_GRACE_SECS + 1` seconds) before sending. `shutdown`, `halt`, and `poweroff` return an error; `reboot` remains unsupported. The host ignores old `GuestToHost::ShutdownRequest` frames for wire compatibility.

**Suspend flow (end-to-end):**
```
Guest: suspend -> capsem-sysutil -> vsock:5004 -> capsem-process
  capsem-process: reads SuspendRequest -> sends ProcessToService::SuspendRequested to service
  capsem-process: saves VM state and exits cleanly
  capsem-service: marks persistent VM suspended for resume
```

### capsem-net-proxy

Listens on localhost:10443 inside the guest. iptables redirects all port 443 traffic here. Each connection is bridged to host vsock:5002 where the network intercept handles TLS termination, protocol parsing, and handoff to the security engine.

### capsem-mcp-server

Guest MCP relay. Reads MCP JSON-RPC on stdin/stdout and carries it to the host MITM MCP endpoint as framed records over vsock:5002.

## Storage modes

Selected by kernel cmdline `capsem.storage=virtiofs` (default) or absence (block mode).

**VirtioFS mode** (default):
```
~/.capsem/sessions/{id}/
  system/rootfs.img    # ext4 loopback (2GB sparse) -- overlayfs upper
  workspace/           # VirtioFS files for /root (host-visible)
  auto_snapshots/      # Rolling ring buffer (12 APFS clones, 5min interval)
```

Boot sequence: profile-selected read-only rootfs asset -> VirtioFS mount -> loopback ext4 -> overlayfs -> bind-mount workspace.

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
4. Host network intercept terminates TLS using per-domain minted certs (signed by static Capsem CA)
5. Host parses HTTP/model facts into a `SecurityEvent` and calls the shared security engine
6. Runtime materialization forwards allowed bytes to upstream
7. Logging plugins produce ledger-safe event output for the logger DB

### Network/security policy

- Corp config owns enterprise constraints, reporting endpoints, and locked
  rule/plugin policy.
- Profile config owns VM assets, MCP config, rules, detections, plugins, and
  defaults for sessions created from that profile.
- Settings config owns UI/app preferences only.
- All enforcement and detection compiles into one `SecurityRuleSet` over
  `SecurityEvent`; there is no domain-policy, HTTP-policy, or MCP-policy
  decision provider.
- Credential capture/injection belongs to the credential broker plugin.
  Durable ledger materialization belongs to the logger DB boundary after
  logging plugins such as `log_sanitizer` produce ledger-safe events. Network
  formatters, service routes, frontend transforms, and debug harnesses must not
  implement credential handling or logged-data caches.

### Logger DB boundary

`capsem-logger` owns SQLite connections and storage mechanics. Routes,
service code, MCP helpers, UI handlers, and benchmarks must not call
`rusqlite::Connection::open` or `DbReader::open` directly and must not maintain
their own telemetry/security projection caches. They call a logger DB object to
run queries and writes.

The DB layer owns connection threads, `mem`/disk table layout, batching, flush,
rehydration, WAL tuning, and future FTS5/search. It does not own product route
semantics by hardcoding route-specific helper methods in `DbWriter`; callers may
own query intent while the DB object owns execution. Missing ledger tables or
columns are schema-contract failures, not empty data.

### MITM CA

- Static CA: `config/capsem-ca.key` + `config/capsem-ca.crt` (ECDSA P-256)
- Baked into rootfs via `update-ca-certificates` + certifi patch
- Guest trusts it via system store + env vars (`REQUESTS_CA_BUNDLE`, `NODE_EXTRA_CA_CERTS`, `SSL_CERT_FILE`)

## Ephemeral VM model (invariants)

**VirtioFS mode**: fresh workspace + sparse rootfs.img per session. Host creates empty dirs, guest formats on first boot.

**Block mode**: `mke2fs` runs unconditionally at boot. Overlay upper is always tmpfs.

**Sessions run profiles.** Session workspace and overlay state are session
state; image contents come from the profile asset contract. Never make the
overlay upper layer a hidden image-authoring rail. To add packages, edit the
profile-owned package files under `config/profiles/<id>/` and rebuild through
the profile-derived asset rail.

**Fork images** extend the session model with reusable templates. `capsem fork
<session> <image-name>` snapshots a session via APFS clonefile. Forks stay tied
to their profile asset contract. Deleting any image is always safe; asset
cleanup protects referenced profile assets.

## Installation and service lifecycle

Release packages are the primary install entry point. Local development uses
the same package rail as CI: build the package, pass a manifest override, and
let the package install service files plus manifest metadata.

Package install handles service registration and manifest placement. Profile
configuration handles security rules, plugins, MCP, assets, and packaged root
content; credentials are brokered at runtime.

**Install layout** (`~/.capsem/`):
- `bin/` -- capsem, capsem-service, capsem-process, capsem-mcp, capsem-gateway, capsem-tray
- `assets/` -- manifest.json and profile-selected VM assets such as `vmlinuz`,
  `initrd.img`, and EROFS rootfs images
- `run/` -- service.sock, service.pid, gateway.token, gateway.port, gateway.pid, instances/{id}.sock

**Service registration**: LaunchAgent `com.capsem.service` (macOS) or systemd user unit `capsem.service` (Linux). KeepAlive/Restart=always. Service auto-launches gateway and tray as companion processes, passing `--parent-pid` so companions self-exit when the service dies (see capsem-guard, `/dev-rust-patterns` lesson 18).

**Auto-launch cascade**: capsem-service starts -> spawns capsem-gateway (port 19222) + capsem-tray. All three are separate processes.

**Self-update**: `capsem update` checks the release-channel health index,
downloads verified binary installers, prints the package-manager apply command
for audit, executes it with `--yes`, materializes VM assets from URL-shaped
manifest sources, and reports manifest origin/hash plus update availability
through service status. Background update-check cache (`update-check.json`, 24h
TTL) refreshes on ordinary CLI commands.

Key source files: `crates/capsem/src/paths.rs`,
`crates/capsem/src/service_install.rs`, `crates/capsem/src/update.rs`, and
`crates/capsem/src/uninstall.rs`.

## Key source files

Read `references/key-files.md` for the full annotated source map.

## Tauri v2 reference

Read `references/tauri-v2.md` for Tauri v2 patterns. capsem-app is a thin webview shell -- only 2 IPC commands (`open_url`, `check_for_app_update`). All VM operations route through the gateway.

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
