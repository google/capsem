---
title: Service Architecture
description: The multi-binary daemon model -- how Capsem's host and guest binaries work together.
sidebar:
  order: 0
---

Capsem uses a service-oriented architecture with multiple cooperating binaries. Every VM operation flows through a single path: client -> service -> per-VM process -> guest.

## Host binaries

Seven binaries run on the host machine. They are installed to
`~/.capsem/bin/` by the platform package or source install flow.

| Binary | Role | Communication |
|--------|------|---------------|
| **capsem** | CLI client | HTTP over UDS to service |
| **capsem-service** | Background daemon | Axum HTTP over UDS (`~/.capsem/run/service.sock`) |
| **capsem-process** | Per-VM process | Spawned by service, MessagePack over UDS |
| **capsem-mcp** | MCP server for AI agents | stdio (rmcp), HTTP over UDS to service |
| **capsem-mcp-aggregator** | External MCP server connections | NDJSON over stdin/stdout, spawned by capsem-process |
| **capsem-gateway** | HTTP/WebSocket gateway | TCP port 19222, proxies to service UDS |
| **capsem-tray** | System tray | Polls gateway for VM status |

Additionally, **capsem-app** is a thin Tauri webview shell (desktop GUI). It connects to the gateway at `http://127.0.0.1:19222` and has no direct VM logic -- all operations route through the gateway to the service.

## Guest binaries

Five binaries run inside each Linux VM, cross-compiled for `aarch64-unknown-linux-musl` and `x86_64-unknown-linux-musl`. All are deployed chmod 555 (read-only).

| Binary | Role | Vsock port |
|--------|------|------------|
| **capsem-pty-agent** | PTY bridge, control channel, exec, file I/O, kernel audit stream | 5000 (control), 5001 (terminal), 5005 (exec), 5006 (audit) |
| **capsem-net-proxy** | Redirects HTTPS to host MITM proxy | 5002 |
| **capsem-dns-proxy** | Redirects DNS queries to the host DNS policy/resolver path | 5007 |
| **capsem-mcp-server** | Guest MCP stdio-to-framed-vsock relay | 5002 |
| **capsem-sysutil** | Lifecycle multi-call (shutdown/halt/poweroff/reboot/suspend) | 5004 |

## Communication diagram

All clients route through capsem-service. There is no direct VM boot from any other binary.

```mermaid
graph TD
    subgraph Clients
        CLI["capsem (CLI)"]
        MCP["capsem-mcp (MCP)"]
        GW["capsem-gateway (TCP:19222)"]
    end

    subgraph "UI Layer"
        APP["capsem-app (Tauri)"]
        TRAY["capsem-tray"]
    end

    APP -->|HTTP| GW
    TRAY -->|HTTP| GW

    CLI -->|HTTP/UDS| SVC
    MCP -->|HTTP/UDS| SVC
    GW -->|HTTP/UDS| SVC

    SVC["capsem-service (daemon)"]

    SVC -->|"MessagePack/UDS"| PROC["capsem-process (per-VM)"]

    PROC -->|"NDJSON/stdio"| AGG["capsem-mcp-aggregator"]
    AGG -->|"HTTP/SSE"| EXT["External MCP servers"]

    subgraph "Linux VM (guest)"
        AGENT["capsem-pty-agent"]
        NETPROXY["capsem-net-proxy"]
        DNSPROXY["capsem-dns-proxy"]
        MCPGW["capsem-mcp-server"]
        SYSUTIL["capsem-sysutil"]
    end

    PROC -->|"vsock:5000,5001,5005,5006"| AGENT
    PROC -->|"vsock:5002"| NETPROXY
    PROC -->|"vsock:5007"| DNSPROXY
    PROC -->|"vsock:5002"| MCPGW
    PROC -->|"vsock:5004"| SYSUTIL
```

## IPC protocol stack

Each layer uses a different protocol optimized for its role:

| Layer | Protocol | Socket |
|-------|----------|--------|
| Frontend/Tray -> gateway | HTTP/1.1 over TCP | `127.0.0.1:19222` (Bearer token auth) |
| Gateway -> service | HTTP/1.1 over UDS | `~/.capsem/run/service.sock` |
| CLI/MCP -> service | HTTP/1.1 over UDS | `~/.capsem/run/service.sock` |
| Service -> process | MessagePack over UDS | `~/.capsem/run/instances/{id}.sock` |
| Process -> guest | Binary frames over vsock | Ports 5000, 5001, 5002, 5004, 5005, 5006, 5007 |

### Vsock port assignments

| Port | Purpose | Binary |
|------|---------|--------|
| 5000 | Control messages (resize, heartbeat, exec, file I/O) | capsem-pty-agent |
| 5001 | Terminal data (PTY I/O) | capsem-pty-agent |
| 5002 | MITM proxy and framed guest MCP endpoint | capsem-net-proxy, capsem-mcp-server |
| 5004 | Lifecycle commands (shutdown/suspend) | capsem-sysutil |
| 5005 | Exec output (direct child stdout) | capsem-pty-agent |
| 5006 | Kernel audit stream | capsem-pty-agent |
| 5007 | DNS proxy queries | capsem-dns-proxy |

## Service lifecycle

### Auto-launch cascade

When the service starts, it spawns two companion processes:

1. **capsem-gateway** -- TCP gateway on port 19222
2. **capsem-tray** -- system tray menu bar icon

All three are separate OS processes. If the service crashes, the LaunchAgent/systemd restarts it automatically.

### Service registration

| Platform | Mechanism | Unit |
|----------|-----------|------|
| macOS | LaunchAgent | `~/Library/LaunchAgents/com.capsem.service.plist` |
| Linux | systemd user unit | `~/.config/systemd/user/capsem.service` |

Both are configured for auto-restart (`KeepAlive`/`Restart=always`) and run-at-login.

### CLI auto-launch

The CLI (`capsem`) auto-launches the service if it's not running. On every service-dependent command:

1. Check socket connectivity
2. Try service manager (LaunchAgent/systemd)
3. Fall back to direct spawn
4. Poll socket for up to 5 seconds

## Per-VM process isolation

Each running VM gets its own `capsem-process` child. This provides security isolation:

- **Minimal environment**: service uses `env_clear()` before spawn -- API keys and tokens from the user's shell never reach the process
- **Socket permissions 0600**: only the owning user can connect to per-VM sockets
- **Session directory 0700**: contains workspace, system, serial.log, session.db
- **No guest-triggered exit**: control channel errors cause loop exit, not `process::exit()`
- **VirtioFS boundary**: only `session_dir/guest/` is shared -- host-only files (session.db, serial.log, snapshots, checkpoints) are outside the share
- **MCP aggregator isolation**: external MCP server connections run in a separate subprocess (`capsem-mcp-aggregator`) with only network access -- no VM, database, or filesystem access. See [MCP Aggregator](/architecture/mcp-aggregator/) for details.

## Service HTTP API

The service exposes a REST API over UDS. The gateway proxies this transparently.

| Method | Path | Purpose |
|--------|------|---------|
| POST | `/vms/create` | Create a new VM (`persistent: true` for named VMs) |
| GET | `/vms/list` | List all VMs (running + stopped persistent) |
| GET | `/vms/{id}/info` | VM details (config, status, persistent) |
| POST | `/vms/{id}/exec` | Execute command, return stdout/stderr/exit_code |
| POST | `/run` | One-shot: provision + exec + destroy |
| POST | `/vms/{id}/stop` | Stop VM (persistent: preserve; ephemeral: destroy) |
| POST | `/vms/{id}/resume` | Resume a stopped persistent VM |
| POST | `/vms/{id}/save` | Convert ephemeral to persistent |
| POST | `/purge` | Kill all temp VMs (`all: true` includes persistent) |
| POST | `/vms/{id}/files/write` | Write file to guest |
| POST | `/vms/{id}/files/read` | Read file from guest |
| GET | `/vms/{id}/logs` | Serial/boot logs |
| POST | `/vms/{id}/inspect` | SQL query against session.db |
| DELETE | `/vms/{id}/delete` | Destroy VM and wipe state |
| POST | `/vms/{id}/pause` | Suspend VM to disk (persistent only) |
| POST | `/vms/{id}/fork` | Fork VM into reusable image |
| GET | `/stats` | Full telemetry dump (all sessions) |
| POST | `/reload-config` | Hot-reload settings from disk |

## Installation

Install registers the service and places host binaries under `~/.capsem/bin/`.
The service owns asset resolution and reports missing/downloading/ready state
to the UI and CLI. Provider credentials are configured in normal user/corp
settings or brokered from runtime security events; there is no setup wizard
authority path.

### Install layout

```
~/.capsem/
  bin/                 capsem, capsem-service, capsem-process, capsem-mcp, capsem-gateway, capsem-tray
  assets/              manifest.json, v{VERSION}/{vmlinuz, initrd.img, rootfs.erofs}
  run/                 service.sock, service.pid, gateway.token, gateway.port, instances/
  update-check.json    Self-update cache (24h TTL)
  user.toml            User settings
  corp.toml            Enterprise config (optional)
```

### Self-update

`capsem update` checks GitHub for new asset versions, downloads in background, cleans up old versions. Binary swap is handled by the platform package manager (DMG/deb).

## Rust crate architecture

| Crate | Type | What |
|-------|------|------|
| `capsem-core` | lib | All shared business logic (VM, network, policy, telemetry, config) |
| `capsem-service` | bin | Daemon. Axum HTTP over UDS, spawns/manages capsem-process children |
| `capsem-process` | bin | Per-VM. Boots VM via capsem-core, bridges vsock, job store |
| `capsem` | bin | CLI. HTTP over UDS to service, direct UDS to process for shell |
| `capsem-mcp` | bin | MCP server (stdio). rmcp crate, bridges tool calls to service |
| `capsem-mcp-aggregator` | bin | Isolated subprocess. Manages external MCP server connections via NDJSON |
| `capsem-gateway` | bin | HTTP gateway. Axum on TCP:19222, Bearer auth, WebSocket terminal relay |
| `capsem-app` | bin | Thin Tauri webview. Points at gateway, bundles frontend/dist as fallback |
| `capsem-tray` | bin | System tray. Polls gateway, shows VM status |
| `capsem-agent` | bin(5) | Guest binaries (pty-agent, net-proxy, dns-proxy, mcp-server, sysutil) |
| `capsem-logger` | lib | Session DB schema, queries, async writer |
| `capsem-proto` | lib | Shared protocol types (host-guest, service-process IPC) |
