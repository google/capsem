---
name: site-architecture
description: Capsem system architecture: service, per-VM processes, CLI, guest agent, vsock, network proxy. Use when you need the design to write, review, or debug across components.
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

## Reference routing

- Read `references/service-and-guest-protocols.md` before changing the
  service/process topology, CLI or MCP execution paths, gateway/service routes,
  host IPC, vsock framing or ports, or any guest agent binary.
- Read `references/storage-network-and-lifecycle.md` before changing VM storage
  or forks, network interception, security or logger ownership, ephemeral
  sessions, installation, service registration, companion launch, or updates.
- Read `references/crate-and-privilege-model.md` before moving responsibilities
  between crates, changing capsem-process permissions or environment, changing
  session/socket/share boundaries, or changing MITM CA-key handling.
- Read `references/key-files.md` before locating the implementation owner for
  an architectural change; it is the full annotated source map.
- Read `references/tauri-v2.md` before changing capsem-app or its Tauri v2
  configuration and IPC. The app remains a thin webview shell: only
  `open_url` and `check_for_app_update`; VM operations route through gateway.

## Core invariants

- All VM operations route through `capsem-service` to one `capsem-process` per
  session. No other host binary boots a VM or talks to guest vsock directly.
- The service and gateway expose an explicit route table. Unknown routes return
  404; do not add compatibility aliases or generic forwarding. Every public
  method/path pair is approval-locked in `config/public-surface.toml`.
- Terminal bytes, control messages, lifecycle requests, and exec output use
  distinct vsock responsibilities. Preserve the port and framing contract in
  `references/service-and-guest-protocols.md`.
- The guest is air-gapped: it has no real NIC, DNS, or direct internet. HTTPS
  reaches the host only through the guest net proxy and the MITM/security path.
- Corp config owns enterprise constraints; profiles own VM assets and runtime
  policy; settings own UI preferences. All enforcement and detection compiles
  into one `SecurityRuleSet` over `SecurityEvent`.
- Credential capture/injection belongs to the credential broker. Durable
  ledger storage belongs to `capsem-logger`; routes, MCP helpers, UI handlers,
  benchmarks, and network formatters must not open SQLite or own projection
  caches. Missing tables or columns are schema failures, never empty data.
- Sessions run profiles. Workspace and overlay bytes are session state, never
  a hidden image-authoring rail; package changes go through profile-owned
  inputs and the profile-derived asset build.
- `capsem-process` stays low privilege: a cleared allowlisted environment,
  0600 sockets, a 0700 session directory, read-only assets and guest binaries,
  and only `session_dir/guest/` shared with the VM.
- capsem-app contains no VM logic or `capsem-core` dependency. Gateway and tray
  are service-owned companions and must self-exit with their parent.

## Key source files

Read `references/key-files.md` for the full annotated source map.

## Tauri v2 reference

Read `references/tauri-v2.md` for Tauri v2 patterns. capsem-app is a thin webview shell -- only 2 IPC commands (`open_url`, `check_for_app_update`). All VM operations route through the gateway.
