---
name: dev-capsem
description: Capsem project overview and navigation. Use when you need to understand what Capsem is, how the codebase is organized, which crate does what, or which skill to consult for a specific area. This is the map of the project -- start here when orienting on any task.
---

# Capsem

Capsem sandboxes AI agents in air-gapped Linux VMs on macOS using Apple's Virtualization.framework (with KVM for Linux). Runs as a daemon service (like Docker). Built with Rust and Astro.

## Crate map

| Crate | What | Key modules |
|-------|------|-------------|
| `capsem-core` | Shared library. All business logic lives here. | `vm/` (machine, config, vsock, serial), `net/` (MITM proxy, policy, CA, SSE), `mcp/` (gateway, tools, policy), `hypervisor/` (Apple VZ, KVM), `image.rs` (ImageRegistry, fork/clone) |
| `capsem-service` | Daemon service. Axum HTTP over UDS, VM lifecycle. | `main.rs` (routes, IPC), `api.rs` (request/response types) |
| `capsem-process` | Per-VM process. Boots VM, bridges vsock, job store. | `main.rs` (vsock setup, IPC handler) |
| `capsem` | CLI client. HTTP over UDS to service. | `main.rs` (create, resume, shell, list, exec, run, stop, delete, persist, purge, info, logs, restart, version, doctor, fork, image) |
| `capsem-mcp` | MCP server for AI agents. Stdio, bridges to service. | `main.rs` (rmcp handler, UDS client) |
| `capsem-gateway` | TCP-to-UDS HTTP gateway. Frontend + tray connect through this. | `main.rs` (Axum router), `proxy.rs`, `status.rs`, `terminal.rs`, `auth.rs` |
| `capsem-app` | Optional Tauri GUI shell. IPC commands, state. | `commands.rs`, `state.rs`, `cli.rs` |
| `capsem-agent` | Guest binaries. Cross-compiled for aarch64/x86_64-linux-musl. | `main.rs` (PTY agent + file I/O), `net_proxy.rs` (TCP relay), `mcp_server.rs` (MCP relay) |
| `capsem-logger` | Session DB schema, queries, async writer. | `schema.rs`, `writer.rs`, `events.rs` |
| `capsem-proto` | Shared protocol types. | `ipc.rs` (ServiceToProcess/ProcessToService), `lib.rs` (HostToGuest/GuestToHost) |

Rule: if logic could be reused or tested without a specific crate, it belongs in `capsem-core`.

## Directory map

| Path | What | Skill |
|------|------|-------|
| `crates/` | Rust workspace | `/site-architecture` |
| `frontend/` | Astro 5 + Svelte 5 + Tailwind v4 + Preline | `/frontend-design` |
| `site/` | Starlight documentation site | `/site-infra` |
| `src/capsem/builder/` | Python image builder CLI | `/build-images` |
| `guest/config/` | Guest TOML configs | `/build-images` |
| `guest/artifacts/` | capsem-init, bashrc, diagnostics | `/dev-capsem-doctor`, `/build-initrd` |
| `assets/` | Built VM assets (gitignored, per-arch) | `/build-images` |
| `skills/` | AI agent skills | `/dev-skills`, `/meta-organize-skills` |
| `config/` | defaults.toml, CA keypair | `/site-architecture` |
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
| `/dev-installation` | Setup wizard, service registration, self-update, install tests |
| `/dev-setup` | New developer onboarding |
| `/dev-skills` | Skills system internals |

### Subsystems
| Skill | When |
|-------|------|
| `/dev-mitm-proxy` | MITM proxy, SSE parsing, telemetry |
| `/dev-mcp` | MCP gateway, tool routing |
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
| `/site-architecture` | System architecture, Tauri, key files |
| `/site-infra` | Astro Starlight docs site |

## Communication paths

```
AI Agent  -> capsem-mcp (stdio) -> HTTP/UDS -> capsem-service -> capsem-process -> vsock -> guest
User CLI  -> capsem (HTTP/UDS)  -> capsem-service -> capsem-process -> vsock -> guest
Tauri GUI -> Tauri IPC          -> capsem-core -> vsock -> guest
Guest HTTPS -> iptables -> vsock:5002 -> Host MITM proxy -> upstream
Guest MCP   -> vsock:5003 -> Host MCP gateway -> external MCP servers
```

Vsock ports: 5000 (control), 5001 (terminal), 5002 (MITM), 5003 (MCP), 5005 (exec output).

## Config hierarchy

1. Corp config (`/etc/capsem/corp.toml`) -- highest priority, MDM-distributed
2. User config (`~/.capsem/user.toml`) -- user overrides
3. Settings registry (`config/defaults.toml`) -- compiled-in defaults

## Key invariants

- Guest VM is air-gapped. No real NIC, no real DNS, no direct internet.
- Guest binaries are read-only (chmod 555). Rootfs mounted read-only.
- **Everything is ephemeral unless asked otherwise.** VMs are temporary by default (destroyed on exit). Only named VMs (`capsem create -n <name>`) are persistent -- their workspace and rootfs overlay survive stops and can be resumed. `capsem create` is always detached; `capsem shell` is the interactive entry point (bare `capsem shell` = temp VM + auto-destroy).
- The binary must be codesigned with `com.apple.security.virtualization`.
- `capsem-core` owns all business logic. App crate and agent crate are thin shells.
- **Fork images are first-class objects.** `capsem fork <vm> <image-name>` snapshots a VM into a reusable template. `capsem create --image <name>` boots from it. Images depend only on a base squashfs version (flat genealogy -- no image-to-image deps). Asset cleanup protects squashfs versions referenced by any image. Images live in `~/.capsem/images/`.
