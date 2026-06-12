---
name: dev-capsem
description: Capsem project overview and navigation. Use when you need to understand what Capsem is, how the codebase is organized, which crate does what, or which skill to consult for a specific area. This is the map of the project -- start here when orienting on any task.
---

# Capsem

Capsem sandboxes AI agents in air-gapped Linux VMs on macOS using Apple's Virtualization.framework (with KVM for Linux). Runs as a daemon service (like Docker). Built with Rust and Astro.

## Crate map

| Crate | What | Key modules |
|-------|------|-------------|
| `capsem-core` | Shared library. All business logic lives here. | `vm/` (machine, profile, vsock, serial), `net/` (network intercept, CA, SSE/model parsing), `security_engine/` (CEL rules, plugins, decisions), `mcp/` (gateway, tools), `hypervisor/` (Apple VZ, KVM), `image.rs` (ImageRegistry, fork/clone) |
| `capsem-service` | Daemon service. Axum HTTP over UDS, VM lifecycle. | `main.rs` (routes, IPC), `api.rs` (request/response types) |
| `capsem-process` | Per-VM process. Boots VM, bridges vsock, job store. | `main.rs` (vsock setup, IPC handler) |
| `capsem` | CLI client. HTTP over UDS to service. | `main.rs` (create, resume, shell, list, exec, run, stop, delete, persist, purge, info, logs, restart, version, doctor, fork, image) |
| `capsem-mcp` | MCP server for AI agents. Stdio, bridges to service. | `main.rs` (rmcp handler, UDS client) |
| `capsem-mcp-aggregator` | Low-privilege subprocess. Connects to external MCP servers and routes tool calls. Communicates with `capsem-process` via length-prefixed msgpack on stdio. No VM / DB / FS access. | `main.rs` (frame loop, server manager) |
| `capsem-mcp-builtin` | Stdio MCP server subprocess exposing built-in tools: HTTP (fetch, grep, headers) and file/snapshot (when `CAPSEM_SESSION_DIR` is set). Managed by the aggregator. | `main.rs` (rmcp handler) |
| `capsem-gateway` | TCP-to-UDS HTTP gateway. Frontend + tray connect through this. | `main.rs` (Axum router), `proxy.rs`, `status.rs`, `terminal.rs`, `auth.rs` |
| `capsem-app` | Thin Tauri webview shell. Points at gateway (`http://127.0.0.1:19222`). 2 IPC commands: `open_url`, `check_for_app_update`. Bundled `frontend/dist` as offline fallback. Crate name matches directory; binary is `capsem-app`. | `main.rs` |
| `capsem-tray` | System tray. Polls gateway for VM status, quick actions (open dashboard, quit). | `main.rs`, `menu.rs` |
| `capsem-agent` | Guest binaries. Cross-compiled for aarch64/x86_64-linux-musl. | `main.rs` (PTY agent + file I/O), `net_proxy.rs` (TCP relay), `mcp_server.rs` (MCP relay), `sysutil.rs` (guest suspend helper; in-VM shutdown disabled) |
| `capsem-logger` | Session DB schema, queries, async writer. | `schema.rs`, `writer.rs`, `events.rs` |
| `capsem-proto` | Shared protocol types. | `ipc.rs` (ServiceToProcess/ProcessToService), `lib.rs` (HostToGuest/GuestToHost) |
| `capsem-guard` | Companion-process lifecycle primitives: parent-watch + singleton flock. Used by gateway and tray to refuse-standalone, enforce one-instance, and self-exit when the service dies (incl. SIGKILL). | `src/lib.rs` (`install`, `Singleton`, `watch_parent_or_exit`) |

Rule: if logic could be reused or tested without a specific crate, it belongs in `capsem-core`.

## Directory map

| Path | What | Skill |
|------|------|-------|
| `crates/` | Rust workspace | `/site-architecture` |
| `frontend/` | Astro 5 + Svelte 5 + Tailwind v4 + Preline | `/frontend-design` |
| `site/` | Marketing website (Astro + Svelte 5) | `/site-marketing` |
| `docs/` | Documentation site (Astro Starlight) | `/site-infra` |
| `src/capsem/builder/` | Python image builder CLI | `/build-images` |
| `guest/artifacts/` | capsem-init, bashrc, diagnostics | `/dev-capsem-doctor`, `/build-initrd` |
| `assets/` | Built VM assets (gitignored, per-arch) | `/build-images` |
| `graphics/` | Brand icons and app icons (source of truth) | `/dev-capsem` |
| `skills/` | AI agent skills | `/dev-skills`, `/meta-organize-skills` |
| `config/` | Profile/corp/admin source config and payloads | `/site-architecture`, `/build-images` |
| `scripts/` | preflight, integration test, doctor session | `/release-process` |

## Skill map

When working on a specific area, consult the relevant skill:

### Development
| Skill | When |
|-------|------|
| `/dev-just` | Which just recipe to run |
| `/dev-testing` | Test policy, TDD, coverage |
| `/dev-debugging` | Bug investigation workflow |
| `/dev-rust-patterns` | Async, cross-compile, error handling |
| `/dev-capsem-doctor` | In-VM diagnostic suite |
| `/dev-installation` | Package install, service registration, self-update, install tests |
| `/dev-setup` | New developer onboarding |
| `/dev-skills` | Skills system internals |

### Subsystems
| Skill | When |
|-------|------|
| `/dev-mitm-proxy` | MITM proxy, SSE parsing, telemetry |
| `/dev-mcp` | Guest MCP endpoint, tool routing |
| `/dev-testing-hypervisor` | KVM, Apple VZ, VirtioFS |
| `/dev-testing-vm` | In-VM tests, session inspection, fixtures |
| `/dev-testing-frontend` | vitest, visual verification |

### Build & release
| Skill | When |
|-------|------|
| `/build-images` | capsem-builder, guest config, rootfs |
| `/build-initrd` | Guest binary repack, fast iteration |
| `/release-process` | Release, CI, signing, docs, changelog |

### Frontend & site
| Skill | When |
|-------|------|
| `/frontend-design` | Design system, colors, Preline, Svelte 5 runes |
| `/site-architecture` | System architecture, service daemon, gateway, key files |
| `/site-infra` | Astro Starlight docs site |

## Communication paths

```
AI Agent    -> capsem-mcp (stdio)      -> HTTP/UDS -> capsem-service -> capsem-process -> vsock -> guest
User CLI    -> capsem (HTTP/UDS)       -> capsem-service -> capsem-process -> vsock -> guest
Desktop UI  -> capsem-gateway (TCP)    -> HTTP/UDS -> capsem-service -> capsem-process -> vsock -> guest
Tray app    -> capsem-gateway (TCP)    -> HTTP/UDS -> capsem-service -> capsem-process -> vsock -> guest
Guest HTTPS -> iptables -> vsock:5002  -> Host MITM proxy -> upstream
Guest MCP   -> framed vsock:5002      -> MITM MCP endpoint -> external MCP servers
```

Vsock ports: 5000 (control), 5001 (terminal), 5002 (MITM + framed guest MCP), 5004 (lifecycle/capsem-sysutil), 5005 (exec output).

## Config hierarchy

1. Corp config -- enterprise constraints, reporting endpoints, and locked rule/plugin policy
2. Profile config -- VM assets, rules, detections, MCP, plugins, packaged root, and profile defaults
3. Settings config -- UI/app preferences only

There is no `user.toml` policy rail. A VM boots a profile; profile/corp own
security behavior. Settings are not policy.

## Key invariants

- Guest VM is air-gapped. No real NIC, no real DNS, no direct internet.
- Guest binaries are read-only (chmod 555). Rootfs mounted read-only.
- **Sessions run profiles.** A session is created from a profile. The profile
  selects assets, packaged root files, MCP config, plugins, rules, detections,
  and UI-facing name/description/icon. Session status must reflect profile
  readiness and compatibility.
- The binary must be codesigned with `com.apple.security.virtualization`.
- `capsem-core` owns all business logic. App crate and agent crate are thin shells.
- **Fork images are first-class objects.** `capsem fork <session> <image-name>`
  snapshots a session into a reusable template. Forked images depend on the
  base profile asset set and must remain compatible with the profile contract.

## Installation

Release packages are the primary install path. `just install` builds the same
package shape as CI and invokes it with a manifest override for local
development.

**Install layout** (`~/.capsem/`):
- `bin/` -- capsem, capsem-service, capsem-process, capsem-mcp, capsem-mcp-aggregator, capsem-mcp-builtin, capsem-gateway, capsem-tray
- `assets/` -- manifest.json and profile-selected VM assets such as `vmlinuz`,
  `initrd.img`, and EROFS rootfs images
- `run/` -- service.sock, service.pid, gateway.token, gateway.port, gateway.pid, instances/

**Service registration**: LaunchAgent (macOS: `com.capsem.service`) / systemd user unit (Linux: `capsem.service`). Auto-restarts on crash. See `/dev-installation` for the full wizard flow.
