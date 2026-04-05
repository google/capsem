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
- **capsem-app** (Tauri GUI): optional GUI shell with xterm.js frontend.

**Guest-side:**
- **capsem-init** (`capsem-init`): PID 1, sets up air-gapped networking, launches daemons
- **capsem-agent** (`capsem-pty-agent`): bridges PTY I/O and file operations over vsock to the host
- **capsem-net-proxy** (`capsem-net-proxy`): redirects HTTPS traffic to host MITM proxy via vsock

## Service architecture

```
AI Agent -> capsem-mcp (stdio) -> HTTP over UDS -> capsem-service (daemon)
User     -> capsem CLI          -> HTTP over UDS -> capsem-service (daemon)
                                                       |
                                          capsem-process (per-VM, UDS IPC)
                                                       |
                                              vsock -> capsem-agent (guest)
```

### IPC protocols

| Layer | Protocol | Socket |
|-------|----------|--------|
| CLI/MCP -> service | HTTP/1.1 over UDS | `~/.capsem/run/service.sock` |
| Service -> process | MessagePack over UDS | `~/.capsem/run/instances/{id}.sock` |
| Process -> guest agent | Binary frames over vsock | ports 5000 (control), 5001 (terminal) |

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

```
capsem CLI shell -> capsem-process UDS -> vsock:5001 -> Guest PTY agent -> bash
capsem-service exec -> capsem-process -> vsock:5000 -> Guest agent -> command
```

Terminal I/O flows through vsock port 5001. Structured commands (exec, file I/O) flow through vsock port 5000 (control channel).

Serial console stays active for kernel boot logs. Terminal I/O switches to vsock once the guest agent sends `Ready`.

### Vsock ports

| Port | Purpose |
|------|---------|
| 5000 | Control messages (resize, heartbeat, exec) |
| 5001 | Terminal data (PTY I/O) |
| 5002 | MITM proxy (HTTPS connections) |
| 5003 | MCP gateway (tool routing, NDJSON passthrough) |

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
- **`capsem-app`**: optional Tauri GUI shell. Wires IPC commands to core.
- **`capsem-agent`**: guest binary. PTY bridge + file I/O + net proxy + MCP relay. Cross-compiled for aarch64/x86_64-linux-musl.
- **`capsem-logger`**: session DB schema, queries, async writer.
- **`capsem-proto`**: shared protocol types. `ipc.rs` (ServiceToProcess/ProcessToService), `lib.rs` (HostToGuest/GuestToHost).
