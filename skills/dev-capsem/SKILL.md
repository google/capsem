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
| `capsem-mcp-aggregator` | Low-privilege subprocess. Connects to external MCP servers and routes tool calls. Communicates with `capsem-process` via length-prefixed msgpack on stdio. No VM / DB / FS access. | `main.rs` (frame loop, server manager) |
| `capsem-mcp-builtin` | Stdio MCP server subprocess exposing built-in tools: HTTP (fetch, grep, headers) and file/snapshot (when `CAPSEM_SESSION_DIR` is set). Managed by the aggregator. | `main.rs` (rmcp handler) |
| `capsem-gateway` | TCP-to-UDS HTTP gateway. Frontend + tray connect through this. | `main.rs` (Axum router), `proxy.rs`, `status.rs`, `terminal.rs`, `auth.rs` |
| `capsem-app` | Thin Tauri webview shell. Points at gateway (`http://127.0.0.1:19222`). 2 IPC commands: `open_url`, `check_for_app_update`. Bundled `frontend/dist` as offline fallback. Crate name matches directory; binary is `capsem-app`. | `main.rs` |
| `capsem-tray` | System tray. Polls gateway for VM status, quick actions (open dashboard, quit). | `main.rs`, `menu.rs` |
| `capsem-agent` | Guest binaries. Cross-compiled for aarch64/x86_64-linux-musl. | `main.rs` (PTY agent + file I/O), `net_proxy.rs` (TCP relay), `mcp_server.rs` (MCP relay), `sysutil.rs` (lifecycle multi-call: shutdown/halt/poweroff/reboot/suspend) |
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
| `guest/config/` | Guest TOML configs | `/build-images` |
| `guest/artifacts/` | capsem-init, bashrc, diagnostics | `/dev-capsem-doctor`, `/build-initrd` |
| `assets/` | Built VM assets (gitignored, per-arch) | `/build-images` |
| `graphics/` | Brand icons and app icons (source of truth) | `/dev-capsem` |
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

## Service API endpoint vocabulary

When adding or changing HTTP/UDS endpoints, use explicit path verbs. Do not mix
configuration reads with runtime counters behind a bare `GET`.

| Path word | Meaning |
|-----------|---------|
| `info` | Configuration, metadata, or contract state. No counters. |
| `status` | Runtime/live state, counters, readiness, health, or progress. |
| `list` | Collection of child resources. |
| `latest` | DB-backed latest ledger rows. |
| `evaluate` | Run a supplied fixture through an engine without mutating config. |
| `reload` | Re-read/apply owned config files and push to running VMs when applicable. |
| `edit` | Mutate configuration. |
| `create` | Create a resource. |
| `delete` | Delete a resource. |

Contract discipline:

- HTTP and UDS expose the same route, DTO, and error shape.
- Profile authoring endpoints are profile-addressed:
  `/profiles/{profile_id}/...`.
- Service-global endpoints are only for daemon health, install/assets cache,
  VM runtime state, and DB-backed runtime ledger views.
- VM behavior is not a UI setting. Assets, VM config, rules, detection, MCP,
  skills, credentials/plugins, and other execution behavior belong to profile.
- Settings are UI/app preferences only.
- Corp config owns constraints, locks, and reporting endpoints over profiles.
- MCP tools/resources/prompts are per server:
  `/profiles/{profile_id}/mcp/servers/{server_id}/tools/list`, etc. There is
  no global MCP tool list.
- Plugin documentation lives on the docs site under `/plugins/...`; do not add
  `/plugins/{id}/man` API routes.
- Provider is not a 1.3 profile API object. Credential brokerage and rules own
  that behavior.

UI reflection discipline:

- The UI reads and writes through approved endpoints; it does not keep a second
  configuration model.
- The UI does not rename backend-owned objects or invent explanatory text for
  profile/rule/plugin/MCP/skill/credential/asset config.
- Backend fields such as `name`, `reason`, `description`, `status`, `source`,
  `group`, and validation messages are the copy/meaning source of truth.
- The UI may add presentation-only structure: grouping, sorting, filtering,
  tabs, buttons, icons, empty/loading/error shell states.
- Direct editing controls reflect backend field cardinality: booleans use
  toggles or checkboxes; enums use select boxes, segmented controls, or
  equivalent enum controls; numbers use numeric inputs/sliders/steppers with
  backend constraints; lists use list editors; free text uses text inputs/areas.
- Rich preview/composed widgets are fine when they improve UX, like the settings
  UI already does, but they must read/write the same backend contract fields and
  not create a second source of truth.
- `settings.toml` is the UI settings contract. The profile schema/profile
  endpoints are the profile and VM behavior contract. Rich profile
  editors/previews must round-trip through profile contract fields.
- Profile availability for web, shell, mobile, or other surfaces is
  profile-backed metadata, not UI settings.
- One UI editor surface writes one underlying contract: settings, profile, corp,
  or runtime. Do not build mixed editor surfaces that write multiple ownership
  planes. Read-only dashboards may combine sources only when source labels are
  explicit.
- UI settings are UI/app preferences only. Do not put VM behavior, security
  rules, MCP config, plugin config, credentials, or assets in frontend settings
  stores.

## Config/profile hierarchy

Capsem runs VMs from profiles. Keep the ownership split sharp:

1. Corp config (`/etc/capsem/corp.toml`) -- constraints, locks, and reporting
   endpoints over profiles.
2. Profile config -- VM behavior: assets, VM config, enforcement, detection,
   MCP, skills, credentials/plugins, and default rules.
3. UI settings -- appearance, notifications, and local UI/app preferences only.

## Key invariants

- Guest VM is air-gapped. No real NIC, no real DNS, no direct internet.
- Guest binaries are read-only (chmod 555). Rootfs mounted read-only.
- **Everything is ephemeral unless asked otherwise.** VMs are temporary by default (destroyed on exit). Only named VMs (`capsem create -n <name>`) are persistent -- their workspace and rootfs overlay survive stops and can be resumed. `capsem create` is always detached; `capsem shell` is the interactive entry point (bare `capsem shell` = temp VM + auto-destroy).
- The binary must be codesigned with `com.apple.security.virtualization`.
- `capsem-core` owns all business logic. App crate and agent crate are thin shells.
- **Fork images are first-class objects.** `capsem fork <vm> <image-name>` snapshots a VM into a reusable template. `capsem create --image <name>` boots from it. Images depend only on a base profile rootfs asset (flat genealogy -- no image-to-image deps). Asset cleanup protects rootfs assets referenced by any image. Images live in `~/.capsem/images/`.

## Installation

Installation is service-first. Packages install the binaries and service unit,
then the app/CLI waits for `capsem-service` readiness and reports
profile-owned asset status. Credentials are not collected during install; the
credential-broker plugin observes and brokers them at runtime.

**Install layout** (`~/.capsem/`):
- `bin/` -- capsem, capsem-service, capsem-process, capsem-mcp, capsem-mcp-aggregator, capsem-mcp-builtin, capsem-gateway, capsem-tray
- `assets/` -- manifest.json plus hash-named kernel, initrd, and rootfs assets
- `run/` -- service.sock, service.pid, gateway.token, gateway.port, gateway.pid, instances/

**Service registration**: LaunchAgent (macOS: `com.capsem.service`) / systemd user unit (Linux: `capsem.service`). Auto-restarts on crash. See `/dev-installation` for the full wizard flow.
