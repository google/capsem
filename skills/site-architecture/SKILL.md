---
name: site-architecture
description: Capsem system architecture: service, per-VM processes, CLI, guest agent, vsock, network proxy. Use when you need the design to write, review, or debug across components.
---

# Capsem Architecture

## Security explanations

Use BLUF: start with one or two short sentences stating how the system is
organized. Introduce a component by its process type, ownership, and concrete
responsibilities before describing its pipeline. Describe components directly; do not open with abstract threat
statements or broad security promises. Prefer a compact table of **Process,
Role, Sandboxing rules** over paragraphs repeating each component's job.
Keep clients, libraries, and host/guest processes distinct. Include the SDKs,
Capsem UI, host and guest MCP, proxy/security engine, log writer, expose relay,
and inter-VM network in the relevant diagram and table; do not hide requested
components behind generic labels. Remove prose already explained by the table
or diagram. Keep only necessary legends, boundary rules, and trust limits.
Use one VM for the process/control diagram; use multiple VMs in the separate
network view. Grey groups mean per-VM ownership, not hardware or OS confinement.
Keep expose routers and security engines inside those groups; shared clients,
gateway, coordinator, and per-network switches stay outside. Give the actual VM
hardware boundary a distinct outline. Label the processing worker **Security
engine**, without an extra "proxy worker" label for the same component.
Do not repeat "one per VM" on nodes inside the grey ownership group. Use node
colors consistently for authority after compromise: green for granted-handle
I/O only, yellow for scoped files/credentials/upstreams, orange for VM control
and virtualization, red for full-user or coordinator authority. Explain the
scale once with a legend. Color file-writing ledgers yellow, not descriptor-only
green; do not color unconfined MCP/client processes as scoped workers. Guest
colors describe host authority, while blue outlines identify VM confinement.
Keep ownership, hardware boundaries, and permission colors separate; external
systems remain uncolored. Colors express permission scope, not a vulnerability
probability or a claim that all IPC-authorized operations are OS-confined.
For each sandbox row, specify startup order, OS mechanism, allowed files and
operations, and denials. Do not summarize all workers as Seatbelt/Landlock/
seccomp or imply clients install a sandbox. Flag undefined OS allowlists rather
than inventing them. Distinguish read-only runtime assets from writable OCI roots.
The agreed target routes CLI operations through the authenticated gateway;
track remaining direct-service paths as implementation work.

Use the completed 0.7/#206 model for the requested security explanation,
while recording unverified target boundaries before publication. Show actual
processes, privilege separation, and OS/hardware sandbox boundaries; file
permissions and separate libraries are not substitutes for confinement.
Show per-VM workers and shared/per-network processes, using multiple VMs when
their relationship is the subject of the diagram.
Use labeled arrows to explain how they work together: control IPC, VM control,
traffic, audit storage, private networks, and published ports. Distinguish these
paths visually; split dense diagrams into readable views when needed. A list
of process boxes does not explain their interactions. Use privilege colors
with a legend, and distinguish assignment groups from OS sandbox boundaries.
Show an OCI container inside each VM's hardware boundary. Place host processes
outside that boundary: the private-network process connects the VMs' guest
networking over separate cables, and the published-port relay reaches only its
granted VM port. Never collapse container, VM, and host-process boundaries or
draw a container's direct host-control/vsock access when guest helpers mediate it.
Honor a supplied diagram reference and inspect the rendered diagram in Astro.
For each boundary, state permitted authority, denied access, and remaining
trust assumptions without claiming more than the accepted final contract.

## System overview

For a component pipeline, distinguish internal stages from OS processes. Show
where a decision is enforced, separate runtime bytes from sanitized ledger
records, and identify observation-only events that cannot prevent an operation.
Link to the owning process-boundary explanation instead of repeating it.
Explain an unfamiliar pipeline through one concrete operation and what each
stage does to it; stage names alone do not explain how components work together.
Architecture prose explains the system, not the drawing. Keep visual notation
in a compact legend when necessary; do not mix box/color commentary with runtime behavior.
Show configuration files as explicit inputs with their formats and compilation
destination. Distinguish detection evidence from enforcement actions; use the
same component name in diagrams and tables, including the network analysis engine.

Capsem sandboxes AI agents in air-gapped Linux VMs on macOS using Apple's Virtualization.framework (with a KVM backend for Linux). It runs as a daemon service (like Docker). The system has these layers:

**Host-side:**
- **capsem-service** (daemon): always-running background service. Axum HTTP server over Unix Domain Socket (`~/.capsem/run/service.sock`). Manages VM lifecycle, routes API calls to per-VM processes. It is the only client of a VM owner: container setup (service-owned OCI pull and staging), exposures (`/vms/{id}/exposures`, admitted by the owner as `network.lifecycle` events), and the `capsem.stream.v1` WebSocket at `/vms/{id}/stream` (terminal, streaming exec, container attach) all translate to typed owner IPC here; CLI, TUI, web and SDKs never dial a per-VM socket.
- **capsem-process** (per-VM): one process per sandbox. Boots the VM, bridges vsock connections (terminal + control), manages structured jobs (exec, file I/O) via a job store.
- **capsem** (CLI): user-facing CLI. Sessions are named by the service
  (`vm-N` unless the user supplies a name) and sized by create flags with
  fixed defaults.
  `capsem shell` opens the TUI over existing VMs; it creates nothing. VM
  creation and control go through the service. User-facing documentation calls
  the environment a VM consistently and uses complete CLI command names.
- **@capsem/mcp** (npm MCP server): separately installed stdio server for AI agents. Uses the TypeScript SDK and an explicit gateway URL/token; it has no UDS, VM-runtime, or virtualization access.
- **capsem-gateway** (HTTP gateway): TCP-to-UDS reverse proxy (default port 19222). Bearer token auth, CORS, 10MB body limit. Provides an explicit route table plus `/status` (cached 1s) and `/vms/{id}/stream` (authenticated `capsem.stream.v1` WebSocket upgrade tunneled unchanged to the service, which translates it to typed VM-owner IPC). Unknown routes return 404; the frontend and tray app connect through the gateway. Writes runtime files to `~/.capsem/run/` (gateway.token, gateway.port, gateway.pid).
- **capsem-app** (Tauri GUI): thin webview shell. Connects to gateway at `http://127.0.0.1:19222`. No VM logic, no capsem-core dependency. Only 2 IPC commands: `open_url` (opens URL in system browser) and `check_for_app_update` (Tauri updater). Bundles `web/app/dist` so the app can render the service-unavailable screen when gateway is unreachable.
- **capsem-tray** (system tray): menu-bar companion process. Polls the gateway for VM status, shows running/stopped counts, and provides quick actions (open dashboard, quit). Non-standalone: refuses to run without `--parent-pid` pointing at a live capsem-service, acquires a system-wide singleton lock at `~/.capsem/run/tray.lock` (only one tray ever in the menu bar), and self-exits within 500ms when its parent dies. Contract enforced by `capsem-guard` on the companion side, not the spawner.
- **capsem-guard** (shared library): parent-watch + singleton primitives used by capsem-tray and capsem-gateway. Provides `watch_parent_or_exit`, `Singleton::try_acquire`, and the umbrella `install(parent_pid, lock_path)`. Guarantees companions die with their parent and can't run standalone or as multiple instances -- closes the orphan-accumulation class of bug that `kill_on_drop(true)` alone cannot cover under SIGKILL/OOM/test-harness termination. See `/dev-rust-patterns` lesson 18.

**Guest-side:**
- **capsem-init** (`capsem-init`): PID 1, sets up air-gapped networking, mounts filesystems, deploys guest binaries, launches daemons, writes boot timing JSONL
- **capsem-pty-agent** (`capsem-pty-agent`): main guest agent -- PTY bridge, control channel, exec, file I/O, shutdown handler (see "Guest agent architecture" below)
- **capsem-tun** (`capsem-tun`): guest end of a network cable. One tap per attached network, MAC `capsem_proto::privatelink::mac_of(address)`, MTU 65521, pumping ethernet frames as u16-framed records over that cable's own vsock:5009 connection. The VM owner holds each cable's stream and hands a duplicate to that network's switch on `plug()`. A wire, not a stack: it parses nothing, and every protocol crosses it.
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
- VMs start unplugged. A private network is one confined L2 switch process
  (`capsem-router --network`); `plug()`/`unplug()` connect a VM's cable to it.
  Plugged VMs talk over any protocol. The switch forwards on MAC only and is a
  network, not a security boundary: never add per-flow relays, admission,
  or IP/TCP parsing to it. See `references/storage-network-and-lifecycle.md`.
- Corp config owns enterprise constraints and wins; the user's
  `settings.toml` and built-in defaults supply the rest of the policy; the
  one VM runtime and OCI images own what a guest runs; settings own UI
  preferences. All enforcement and detection compiles
  into one `SecurityRuleSet` over `SecurityEvent`.
- Credential capture/injection belongs to the credential broker. Durable
  ledger storage belongs to `capsem-logger`; routes, MCP helpers, UI handlers,
  benchmarks, and network formatters must not open SQLite or own projection
  caches. Missing tables or columns are schema failures, never empty data.
- Workspace and overlay bytes are session state, never
  a hidden image-authoring rail; package changes go through the runtime
  package set (`/build-images`) or an OCI image under `images/`.
- `capsem-process` stays low privilege: a cleared allowlisted environment,
  0600 sockets, a 0700 session directory, read-only assets and guest binaries,
  and only `session_dir/guest/` shared with the VM.
- capsem-app contains no VM logic or `capsem-core` dependency. Gateway and tray
  are service-owned companions and must self-exit with their parent.

## Key source files

Read `references/key-files.md` for the full annotated source map.

## Tauri v2 reference

Read `references/tauri-v2.md` for Tauri v2 patterns. capsem-app is a thin webview shell -- only 2 IPC commands (`open_url`, `check_for_app_update`). All VM operations route through the gateway.
