# Sprint: Next-Gen Platform

## What

Transform Capsem from a single-VM CLI/GUI into a multi-VM daemon platform (`capsem-service`) with checkpoint/branching, SSH integration, browser-chrome UI, IDE support, and full-text search. Reference design: `tmp/docs/next_gen_v2.md`.

## Why

- Single-VM model is a dealbreaker for power users
- No daemon means no background VMs, no remote access, no IDE integration
- Current UI is a sidebar layout that doesn't scale to multiple VMs
- No shell mode, no SSH, no way for VS Code to connect
- No search over session history
- Frontend needs full Preline rebuild with strict TypeScript and independent tests

## Key Decisions

- Daemon is called **`capsem-service`**, not `capsem-daemon`
- SSH keys are the universal identity (no tokens, no passwords, no separate CA)
- Browser-chrome UI metaphor: tabs = VMs, toolbar = controls, side panel = stats/checkpoints/settings
- Frontend rebuilt from scratch with Preline semantic tokens, no raw Tailwind colors
- Every service runs as its own sandboxed process (crash isolation, sandbox granularity)
- **MCP tools ship with each sprint** -- as capabilities come online, their MCP tools ship immediately
- Shell (interactive CLI) follows daemon foundation
- SSH before UI (SSH enables IDE integration which enables dogfooding)
- **Resource management is load-bearing**: Apple VZ wires guest RAM upfront. N VMs without admission control = host swap death spiral or kernel panic. ResourceManager must exist before multi-VM.

## Resource Management Architecture

Cross-cutting concern that threads through Sprints 1 and 4. Three layers:

### Layer 1: Admission Control (Sprint 1)

Static gate at VM boot time. `ResourceManager` tracks `total_allocated_ram` and `total_allocated_cpu`. Rejects `provision_sandbox` if `requested_ram + current_allocated > system_ram - 8GB`. Simple, prevents the obvious footgun.

### Layer 2: Runtime Pressure Monitoring (Sprint 4)

Continuous monitoring thread in capsem-service that watches host pressure signals:

- **macOS**: `dispatch_source_create(DISPATCH_SOURCE_TYPE_MEMORYPRESSURE)` for memory pressure notifications (warn/critical/normal transitions). `host_statistics64` for page-level stats. `IOReportCopyChannelsInGroup` or `iostat` sampling for disk I/O saturation.
- **Linux**: `/proc/pressure/memory` and `/proc/pressure/io` (PSI -- Pressure Stall Information). Kernel exposes `some` and `full` stall percentages over 10s/60s/300s windows.

When pressure is detected, the monitor emits `ResourcePressure` events to the orchestrator with severity (normal/warning/critical) and type (memory/io/both).

### Layer 3: Auto-Nap (Sprint 4)

The orchestrator reacts to `ResourcePressure` events:

1. **Identify idle VMs**: no terminal activity, no network requests, no tool calls for N minutes (configurable, default 10min). Idle score = time since last activity.
2. **Memory pressure warning**: log it, surface in health endpoint and menu bar.
3. **Memory pressure critical**: auto-checkpoint the highest-idle-score VM, transition to `Napping` state (checkpointed + RAM freed). If still critical after nap, nap the next most idle. Never nap a VM with active terminal/SSH session.
4. **Pressure drops to normal**: auto-resume napped VMs in reverse order (most recently napped first).
5. **User interaction with napped VM** (terminal connect, SSH, MCP exec): instant resume from checkpoint, bump priority so it's last to be napped again.

`Napping` is a distinct state from `Suspended` (user-initiated checkpoint). Napped VMs show a sleep icon in UI/menu bar and auto-wake on interaction.

### Layer 4: Memory Optimization (Sprint 4)

Platform-specific memory savings:

- **KVM**: `madvise(guest_mem, size, MADV_MERGEABLE)` on guest memory allocation. Enables KSM (Kernel Same-page Merging). Since all VMs share the same kernel + rootfs, identical pages are deduplicated by the host kernel. Typical savings: 30-60% for same-base-image VMs. Zero guest changes needed.
- **Apple VZ**: No KSM equivalent. No balloon. No dynamic resize. Mitigation: right-size VM RAM (expose `--ram` flag, default to smaller allocations like 2GB instead of 4GB for secondary VMs), and rely heavily on auto-nap to reclaim RAM from idle VMs entirely.

### Disk I/O Queuing (Sprint 4)

Heavy host-side disk operations that can saturate IOPS:

- Snapshot create/branch/merge (clonefile or recursive copy)
- Rootfs image creation (mke2fs at first boot)
- Checkpoint save/restore (full VM state to disk)

These go through a `DiskWorkQueue` (bounded semaphore, default concurrency=1 for HDD, 2 for SSD -- auto-detected via `statvfs` or IOKit). Guest I/O flows through VirtioFS/virtio-blk and is inherently concurrent -- the queue only governs host-side bulk operations that we control.

## Network & Telemetry Isolation

Each capsem-process is a self-contained sandbox -- MITM proxy, MCP gateway, telemetry recorder, and VirtioFS monitor all run inside the child process, not in capsem-service.

**Why**: VZ's vsock listener is tied to the specific VM instance object. Keeping the proxy in the child process means no multiplexing N VMs over a shared IPC bridge. If one VM's proxy panics, the others are unaffected.

**Per-VM databases**: Already the case today -- each session gets its own `session.db`. In multi-VM, each capsem-process writes exclusively to `~/.capsem/instances/<vm_id>/session.db`. Zero SQLite write contention, perfect event correlation, no cross-contamination.

**MCP gateway**: stays in capsem-process, not centralized. External MCP servers are HTTP (stateless, handle N clients). Per-VM policy evaluation stays in-process. Tool pinning cache is file-based (`~/.capsem/mcp_tool_cache.json`, `flock` for writes) -- shared without a centralized process. No IPC multiplexing needed.

**capsem-service role**: The daemon is an orchestrator and API surface only. It does NOT handle proxy traffic, MCP routing, or telemetry writes. It queries child session DBs on demand (for health endpoint, MCP telemetry tools, cross-VM search via `ATTACH DATABASE`).

## Headless Renderer Architecture

Each capsem-process can spawn a **headless browser subprocess** that the AI agent controls via MCP tools. Think Chrome's multi-process model: each renderer is fully isolated.

### Why

AI agents need to inspect, debug, and interact with web pages -- verify UI changes, scrape documentation, test endpoints. Today this requires a visible browser or screenshots from an external tool. The renderer makes it a first-class VM capability.

### Process Model

```
capsem-process "dev"
    +-- VM (guest)
    +-- MITM proxy
    +-- MCP gateway
    +-- capsem-renderer (headless browser subprocess)
        +-- works on its own process, its own sandbox profile
        +-- no access to host filesystem beyond a scratch dir
        +-- no access to other VMs' renderers
        +-- communicates with capsem-process via UDS or pipe
```

The renderer is a child of capsem-process, not of capsem-service. If the VM stops, its renderer dies. If the renderer crashes, the VM is unaffected.

### Browser Engine

Candidates: headless Chromium (CEF/puppeteer), Servo, or a lightweight engine like `wry`/`tao` (Tauri's own webview layer). Decision deferred to sub-sprint plan. Key requirement: must support headless mode, DOM inspection, screenshot capture, JS evaluation, and network interception.

### MCP Tools (shipped with renderer sprint)

| Tool | Description |
|------|-------------|
| `renderer_navigate` | Navigate to URL, wait for load |
| `renderer_screenshot` | Capture viewport as PNG (returned as base64) |
| `renderer_inspect` | Return DOM snapshot / accessibility tree |
| `renderer_evaluate` | Execute JS in page context, return result |
| `renderer_click` | Click element by selector or coordinates |
| `renderer_type` | Type text into focused element |
| `renderer_network` | List network requests made by the page |
| `renderer_console` | Get console log messages |

### UI Preview (Sprint 10 -- frontend views)

When the Tauri UI wants to show a renderer preview:

- **Screenshot mode** (safe default): renderer sends PNG screenshots to capsem-service via the capsem-process pipe. UI displays as an `<img>`. Zero attack surface.
- **Live preview** (optional, guarded): separate Tauri webview window with maximal sandboxing -- `sandbox` attribute, no `allow-same-origin`, no `allow-scripts` propagation to parent, separate process. The preview webview connects to the renderer's content stream, never to the host UI's DOM. If the sandboxed webview is compromised, it cannot escape to the parent Tauri window.

**Never** render untrusted AI-navigated content in the same webview/process as the Capsem UI chrome. Same principle as Chrome: renderer processes cannot access the browser chrome process.

### Tauri Process Isolation

The Tauri app in Sprint 9-10 needs per-tab process boundaries:

- Each VM tab should use a separate Tauri webview (not a single webview with JS-level tab switching)
- Renderer previews get their own webview with `sandbox` restrictions
- Tab crash = one webview dies, others survive
- This mirrors Chrome's one-process-per-tab model

```
capsem-service (orchestrator)
    |
    +-- capsem-process "dev"      (VM + MITM proxy + MCP gateway + telemetry -> dev/session.db)
    +-- capsem-process "research" (VM + MITM proxy + MCP gateway + telemetry -> research/session.db)
    +-- capsem-process "codex"    (VM + MITM proxy + MCP gateway + telemetry -> codex/session.db)
```

## Daemon Recovery Architecture

capsem-service must survive crashes and restarts without killing running VMs.

### Filesystem as Truth

No centralized SQL for runtime state. The filesystem IS the state:

```
~/.capsem/run/instances/
    dev.json        # static config: allocated RAM, image, ports, vsock CID, PID
    dev.sock        # per-VM Unix Domain Socket (owned by capsem-process)
    research.json
    research.sock
```

Each `capsem-process` child creates its own `.json` + `.sock` on startup. The daemon doesn't need to be running for VMs to exist -- they are self-describing on disk.

### Reconciliation Loop

On capsem-service startup (including after crash):

1. Read `~/.capsem/run/instances/*.json` to discover previously-running VMs
2. For each: ping the corresponding `.sock`
   - **Socket responds**: adopt the running VM into the orchestrator. Reconnect telemetry, resource tracking, pressure monitoring.
   - **Socket dead**: VM crashed. Clean up orphaned `.json` + `.sock`. Transition state to `Stopped`. Log the crash in audit log.
3. Rebuild `ResourceManager` totals from adopted VMs (sum allocated RAM/CPU)
4. Resume pressure monitoring and auto-nap scheduling

### Locking

`flock()` on each `.json` state file to prevent races when multiple CLI commands fire simultaneously during daemon boot. The capsem-process holds a shared lock while running; the daemon takes an exclusive lock only during cleanup of dead instances.

### Why Not SQLite

- SQLite requires the daemon process to be alive to write. If the daemon dies, state is frozen.
- Filesystem state survives any process crash. `ls ~/.capsem/run/instances/` always tells the truth.
- No WAL corruption risk from unclean daemon shutdown.
- Each capsem-process owns its own files -- no cross-process write contention.

## How to Execute

**Each sprint below requires its own sub-sprint.** Before starting work on any sprint, create `tmp/next-gen/sprint-NN/plan.md` and `tmp/next-gen/sprint-NN/tracker.md` with the detailed implementation plan for that sprint. The entries below define scope and deliverables; the sub-sprint plan defines how to build it.

## Sprints

### Sprint 1: capsem-service boots one VM

Thinnest possible daemon that can boot and manage a single VM.

- [ ] `capsem-process` crate: thin Service trait + CancellationToken (grow later)
- [ ] `capsem-service` crate: main.rs (fork/setsid), single-VM orchestrator
- [ ] `~/.capsem/run/instances/` directory: per-VM `<vm_id>.json` (static config: RAM, image, ports, vsock CID, PID) + `<vm_id>.sock` (per-VM UDS owned by capsem-process). Filesystem is the source of truth -- no SQL for runtime state. See "Daemon Recovery Architecture" above.
- [ ] Reconciliation loop on startup: read `instances/*.json`, ping `.sock`, adopt live VMs or clean up dead ones. Rebuild ResourceManager totals from adopted VMs.
- [ ] `flock()` on `.json` state files to prevent races from concurrent CLI commands during daemon boot.
- [ ] ResourceManager (Layer 1): centralized tracking of total_allocated_ram, total_allocated_cpu. Admission control gate -- reject VM boot if `requested_ram + current_allocated > system_ram - 8GB`. Health endpoint reports resource headroom. See "Resource Management Architecture" above.
- [ ] Unix socket listener (`~/.capsem/service.sock`)
- [ ] Health endpoint (`GET /health` with host resources, allocated vs available RAM/CPU)
- [ ] Boot one VM, stop it, shut down cleanly
- [ ] SIGTERM handler: graceful VM shutdown
- [ ] **MCP tools**: `provision_sandbox`, `list_sandboxes`, `shutdown`, `get_status`
- **Verify**: `capsem-service` starts, boots a VM, MCP client can provision + query status + shutdown. Provision request exceeding RAM headroom is rejected with clear error. Kill capsem-service, restart it -- it rediscovers the running VM via reconciliation loop without killing it.

### Sprint 2: CLI + HTTP API

CLI to control capsem-service, HTTP API skeleton.

- [ ] CLI commands: `capsem service start|stop|status`, `capsem start [--name]`, `capsem stop [<id>]`, `capsem list`
- [ ] HTTP API: `GET /health`, `GET /vms`, `GET /{vm_id}/status`, `POST /{vm_id}/stop|pause|resume`
- [ ] **MCP tools**: `pause`, `resume`, `get_logs`, `screenshot`
- **Verify**: `capsem service start` boots daemon, `capsem start --name dev` boots VM, `capsem list` shows it, `capsem stop dev` kills it

### Sprint 3: WebSocket terminal + exec

Terminal bridge and command execution through the daemon.

- [ ] WebSocket terminal bridge (`WS /{vm_id}/terminal`)
- [ ] Exec path through daemon (run command in VM, get stdout/stderr/exit_code)
- [ ] **MCP tools**: `run_exec`, `read_file`, `write_file`, `list_files`
- **Verify**: WebSocket terminal works, MCP client can run commands and read/write files in VM

### Operations matrix (from checkpoint/restore spike)

| Operation | What it does | Mechanism | Needs CPU+memory state? |
|-----------|-------------|-----------|------------------------|
| **Suspend/Resume** | Freeze VM in place, resume later (same VM) | Quiescence + hypervisor save/restore | Yes |
| **Branch** | Copy environment into a new VM | Quiescence + disk copy (reflink), boot fresh VM | No |
| **Rewind** | Roll back to a previous point | Quiescence + restore disk from snapshot, boot fresh VM | No |

All three operations require **guest quiescence** first (see below). Branch and rewind are **disk-only** on both platforms (APFS clonefile / FICLONE, <1ms). Suspend/resume saves full VM state; proven on Apple VZ (730ms round-trip, 54MB for 2GB VM), ~11-15 days to build on KVM.

### Sprint 4: Multi-VM + quiescence

Multi-VM orchestrator and the quiescence protocol that gates all snapshot operations.

**Guest quiescence** brings the VM to a clean state before any disk or memory operation. The host sends `PREPARE_SNAPSHOT` via vsock, the guest agent runs `sync` + `fsfreeze -f /` (flushes dirty pages, halts all filesystem I/O, empties virtio queues), then acks `SNAPSHOT_READY`. After the operation completes, `UNFREEZE` resumes I/O. Same mechanism as QEMU's `guest-fsfreeze-freeze` and cloud snapshot APIs.

- [ ] Multi-VM orchestrator (state machine for N VMs, concurrent boot/stop)
- [ ] VM states: `Booting`, `Running`, `Paused`, `Suspended`, `Napping`, `Stopping`, `Stopped`, `Failed`
- [ ] Agent protocol extension: `PREPARE_SNAPSHOT`, `SNAPSHOT_READY`, `UNFREEZE` messages
- [ ] Agent reconnect-on-broken-pipe (existing retry-on-connect is the template)
- [ ] `CheckpointManager`: orchestrates quiescence flow (send prepare, wait for ready, do operation, unfreeze)
- [ ] **MCP tools**: `list_sandboxes`, `get_status`, `quiesce` (for testing/debugging)
- **Verify**: Boot 3 VMs concurrently. Quiesce VM A (verify `fsfreeze` runs, agent acks). Unfreeze VM A (verify I/O resumes). Kill capsem-service mid-quiesce -> guest auto-unfreezes after timeout.

### Sprint 5: Branch + rewind

Disk-only operations. Both use quiescence to ensure consistency, then copy or restore the workspace filesystem. The target VM cold-boots from the disk -- no CPU/memory state involved.

- [ ] **Branch**: quiesce guest, reflink-copy workspace (APFS clonefile / FICLONE), unfreeze original, boot fresh VM pointing at the copy
- [ ] **Rewind**: quiesce guest, stop VM, restore workspace disk from a named snapshot, boot fresh VM
- [ ] Named disk snapshots: create/list/delete snapshot of workspace state (reflink copy to `~/.capsem/instances/<vm_id>/snapshots/<name>/`)
- [ ] DiskWorkQueue: bounded semaphore for host-side bulk disk ops (branch/rewind). Concurrency auto-detected (1 for HDD, 2 for SSD).
- [ ] Existing snapshot tools (snapshots_*) wired through daemon
- [ ] **MCP tools**: `branch`, `rewind`, `list_snapshots`, `create_snapshot`, `delete_snapshot`
- **Verify**: Branch VM A into VM B (verify VM B boots from copied disk, has same files/packages). Rewind VM A to earlier snapshot (verify filesystem rolled back). Branch during active I/O (quiescence ensures consistency). Concurrent branches queued by DiskWorkQueue.

### Sprint 6: Suspend/resume + auto-nap

Suspend saves full VM state (CPU + memory + devices) and frees RAM. Resume restores to the same VM instance. Auto-nap uses suspend to free RAM under memory pressure.

Apple VZ: proven by spike (730ms round-trip, 54MB for 2GB VM). Linux: deferred to Sprint 6b (~11-15 days, uses crosvm crates).

- [ ] **Suspend** (Apple VZ): quiesce guest, `save_state` (spike code), stop VM, free RAM. `Suspended` state = user-initiated.
- [ ] **Resume** (Apple VZ): `restore_state`, `resume`, wait for agent reconnect, unfreeze.
- [ ] Runtime pressure monitor (Layer 2): background thread watching memory pressure (macOS `DISPATCH_SOURCE_TYPE_MEMORYPRESSURE`, Linux `/proc/pressure/memory`) and disk I/O. Emits `ResourcePressure` events with severity.
- [ ] Auto-nap (Layer 3): on critical memory pressure, auto-suspend the most-idle VM (no activity for N minutes) to free RAM. Auto-resume when pressure drops. Never nap VMs with active sessions. `Napping` = auto-suspend, distinct from user-initiated `Suspended`.
- [ ] Memory optimization (Layer 4): KVM -- `madvise(MADV_MERGEABLE)` on guest memory for KSM. Apple VZ -- right-size RAM (2GB default for secondary VMs), rely on auto-nap.
- [ ] **MCP tools**: `suspend`, `resume`, `inspect_network`, `query_telemetry`, `get_session_summary`, `list_sessions`, `export_telemetry`
- **Verify**: Suspend VM A (verify RAM freed, ~730ms). Resume VM A (verify state restored, agent reconnects, I/O works). Boot a VM that exceeds quota -> most-idle VM auto-napped, new VM boots. Interact with napped VM -> instant resume. KVM: verify KSM (`/sys/kernel/mm/ksm/pages_sharing > 0`).

### Sprint 6b: Linux suspend/resume (deferred)

Linux suspend/resume (~11-15 engineering days). Implement snapshot/restore directly using `kvm-ioctls` (already in our deps) + serde. crosvm crates are **not usable as a library** (monorepo `base` crate conflicts with Tauri's `windows` crate version; `devices` crate requires `minijail` C library). Instead, follow crosvm's *pattern*: the snapshot logic is ~200 lines of KVM ioctls + serde structs. With quiescence emptying virtio queues, device state is trivial (just config + ring indices, no in-flight descriptors).

- [ ] `VcpuSnapshot` struct (serde): GP regs, SP, PC, PSTATE, SIMD regs, system regs via `KVM_GET_ONE_REG`
- [ ] `GicSnapshot` struct (serde): distributor + redistributor + ITS state via `KVM_DEV_ARM_VGIC_GRP_*`
- [ ] Guest memory: `MAP_SHARED` mmap dump for instant restore (no byte copy)
- [ ] `VirtioDeviceSnapshot` struct (serde): per-device config + queue indices (queues guaranteed empty by quiescence)
- [ ] `VmHandle::save_state` / `restore_state` for Linux backend
- **Verify**: Suspend Linux VM, restore, verify guest resumes. virtio-fs, vsock, console all functional. Benchmark latency and checkpoint size.

### Sprint 7: Shell (capsem as interactive CLI)

- [ ] `capsem shell [--name <id>]` interactive PTY session
- [ ] Standalone mode: no daemon -> boot VM directly
- [ ] Attach mode: daemon running -> connect to VM terminal via WebSocket
- [ ] Proper termios save/restore, cfmakeraw, SIGWINCH handler
- [ ] Poll loop: stdin -> VM, VM -> stdout
- [ ] Clean shutdown: restore termios on exit/SIGINT
- **Verify**: `capsem shell` gives interactive bash, vim/top/less work, window resize propagates, `exit` returns to host

### Sprint 8: SSH auth + remote API

- [ ] SSH key loading (`~/.capsem/authorized_keys`, auto-populate from `~/.ssh/*.pub`)
- [ ] SshAuthorizedKeysVerifier (custom rustls ClientCertVerifier, SPKI matching)
- [ ] Client-side: ephemeral X.509 cert from SSH key (`ssh-key` + `rcgen`)
- [ ] TLS listener (remote API with mTLS)
- [ ] `capsem authorize <pubkey>`
- [ ] Periodic SSH key rescan on daemon startup (notify, don't auto-add)
- **Verify**: mTLS succeeds with SSH-derived cert, unauthenticated rejected, remote `capsem list` works

### Sprint 9: Menu bar + auto-start + notifications

- [ ] macOS menu bar (NSStatusItem): VM list, state dots, pause/stop/resume per VM
- [ ] Auto-start: LaunchAgent plist, `capsem autostart enable|disable|status`
- [ ] Terminal notifications: OSC sequences -> OS notifications -> menu bar badge
- [ ] Confirmation system: ConfirmationRequest events, menu bar fallback
- [ ] **MCP tools**: `get_tray_status`, `tray_action`, `set_tray_badge`
- **Verify**: Menu bar visible, auto-start survives logout, bell triggers notification

### Sprint 10: MITM SSH + IDE

- [ ] Guest openssh-server in rootfs, capsem-ssh-bridge binary (vsock:5006 -> localhost:22, inbound only)
- [ ] `russh` SSH server in daemon (port 2222, configurable)
- [ ] VM routing via SSH username (`ssh dev@localhost:2222`)
- [ ] SSH MITM: terminate, inspect, policy, log telemetry
- [ ] SSH session recording (`ssh_events` table, terminal I/O, commands, file transfers)
- [ ] `capsem ssh-config [<id>]`
- [ ] VS Code extension skeleton (Start/Stop/Connect/Open Terminal)
- [ ] Security: inbound-only, no SSH client keys in guest
- **Verify**: `ssh dev@localhost:2222` works, VS Code Remote opens, SSH commands logged

### Sprint 11: Frontend foundation (tab shell + Preline)

Full ground-up rebuild. Simplest working tab system first, build down. Tauri process isolation from day one.

- [ ] Preline setup: install, Tailwind v4 plugin, theme tokens (blue primary, purple destructive)
- [ ] TypeScript: strict types for daemon API, independent vitest suite, testable data layer
- [ ] Tauri process isolation: each VM tab uses a separate Tauri webview (not JS-level tab switching). Tab crash = one webview dies, others survive. Same model as Chrome's one-process-per-tab.
- [ ] `BrowserShell.svelte`: root layout (tab bar + toolbar + content area)
- [ ] `TabBar.svelte` + `Tab.svelte`: horizontal tabs, state dots, close, new tab, Cmd+T/W/1-9
- [ ] `Toolbar.svelte`: VM controls, address bar, content mode toggle, actions
- [ ] `tabs.svelte.ts` + `panel.svelte.ts` stores
- [ ] Tab routing: click tab -> show content, new tab -> new tab page
- [ ] **MCP tools**: `open_ui`, `close_ui`, `screenshot_ui`, `get_ui_state`, `navigate_ui`, `resize_ui`
- **Verify**: App boots, tabs switch, Preline tokens everywhere, TS tests pass independently, each tab is a separate webview process

### Sprint 12: Frontend views (terminal + panel + new tab page)

- [ ] `TerminalView.svelte`: per-tab xterm.js, connected to daemon WS
- [ ] `SidePanel.svelte`: toggleable right panel (Stats/Checkpoints/Settings)
- [ ] Stats panel: summary cards + all sub-tabs (network, AI, tools, models, files, snapshots)
- [ ] `CheckpointsPanel.svelte`: checkpoint timeline tree
- [ ] Settings panel: settings tree, MCP section, presets
- [ ] `NewTabPage.svelte`: VM list, create new, first-run setup (absorbs wizard)
- [ ] `StatsBar.svelte`: bottom bar with tokens/tools/cost
- [ ] All stores rewritten: vmStore (multi-VM Map), statsStore (per-VM), wizard absorbed
- **Verify**: Terminal per tab, panel slides, stats populate, new tab page shows VMs

### Sprint 13: Search (FTS5)

No master telemetry database. Two strategies for cross-VM search:

- **Per-session FTS5**: each `session.db` gets its own `event_log_fts` contentless index. Single-session search is a local query. This is the fast path for the active VM's panel search bar.
- **Cross-session search**: capsem-service uses SQLite `ATTACH DATABASE` to query multiple `session.db` files dynamically, or maintains a lightweight FTS5 index in `main.db` that stores (session_id, event_type, snippet) pointers back to session data. No data duplication -- just an index of pointers.

Tasks:

- [ ] Bump AI_BODY_PREVIEW to 512KB
- [ ] Extract user messages from provider parsers (Anthropic, OpenAI, Google)
- [ ] `user_message_text` column in model_calls
- [ ] Per-session FTS5 contentless index (`event_log_fts`) indexing 8 event types
- [ ] `search_events()` reader API with snippet highlighting (single-session)
- [ ] Cross-session search via `ATTACH DATABASE` or `main.db` FTS5 pointer index
- [ ] Settings: `search_enabled`, `index_user_messages`
- [ ] Search bar in UI (per-VM in panel, global in toolbar)
- **Verify**: Search returns results within active session. Global search returns results across all sessions/VMs without a centralized telemetry DB.

### Sprint 14: Polish + enterprise

- [ ] Disk space monitoring + auto-eviction for snapshots
- [ ] OpenTelemetry subscriber (feature-gated)
- [ ] Enterprise policy endpoint (remote corp.toml pull)
- [ ] Audit logging (`~/.capsem/audit.log`)
- [ ] OS-level sandbox profiles per process
- [ ] Corp manifest enforcement (feature gates, locked settings in UI)
- [ ] Full test gate: `just test` + `just run "capsem-doctor"` + `just full-test`

### Sprint 15: Headless renderer

Per-VM headless browser subprocess. The AI agent controls it via MCP tools. See "Headless Renderer Architecture" above.

- [ ] `capsem-renderer` binary: headless browser engine (engine choice in sub-sprint plan -- CEF, Servo, or wry/tao)
- [ ] Subprocess lifecycle: capsem-process spawns renderer on demand, kills on VM stop. Crash-isolated from VM and other renderers.
- [ ] Sandbox profile: own process, no host filesystem access beyond scratch dir, no access to other VMs
- [ ] Communication: UDS or pipe between capsem-process and renderer
- [ ] **MCP tools**: `renderer_navigate`, `renderer_screenshot`, `renderer_inspect`, `renderer_evaluate`, `renderer_click`, `renderer_type`, `renderer_network`, `renderer_console`
- **Verify**: MCP client navigates to URL, gets screenshot, inspects DOM, evaluates JS. Renderer crash does not affect VM. Renderer has no access to host filesystem or other VMs.

### Sprint 16: Renderer UI preview

Integrate headless renderer output into the Tauri UI.

- [ ] Screenshot mode (safe default): renderer sends PNG screenshots via capsem-process pipe. UI displays as `<img>`. Zero attack surface.
- [ ] Live preview (optional, guarded): separate Tauri webview window with maximal sandboxing -- `sandbox` attribute, no `allow-same-origin`, no `allow-scripts` propagation to parent, separate process. Preview webview connects to renderer's content stream, never to host UI DOM.
- [ ] Preview panel in SidePanel or as a dedicated content mode alongside Terminal/Chat
- [ ] Security: untrusted AI-navigated content never shares a process/webview with Capsem UI chrome
- **Verify**: Renderer output visible in UI via screenshot mode. Live preview in sandboxed webview. Compromised preview webview cannot access parent Tauri window. Preview crash does not affect other tabs.

## MCP Tool Delivery Map

| Sprint | MCP Tools Shipped |
|--------|-------------------|
| 1 | provision_sandbox, list_sandboxes, shutdown, get_status |
| 2 | pause, resume, get_logs, screenshot |
| 3 | run_exec, read_file, write_file, list_files |
| 4 | quiesce (debug) |
| 5 | branch, rewind, list_snapshots, create_snapshot, delete_snapshot |
| 6 | suspend, resume, inspect_network, query_telemetry, get_session_summary, list_sessions, export_telemetry |
| 9 | get_tray_status, tray_action, set_tray_badge |
| 11 | open_ui, close_ui, screenshot_ui, get_ui_state, navigate_ui, resize_ui |
| 15 | renderer_navigate, renderer_screenshot, renderer_inspect, renderer_evaluate, renderer_click, renderer_type, renderer_network, renderer_console |

All existing snapshot tools (snapshots_*) wired through daemon in Sprint 5.

## Critical Path

```
Sprint 1 (service boots VM)
    |
    v
Sprint 2 (CLI + API) ---> Sprint 3 (terminal + exec)
    |
    v
Sprint 4 (multi-VM + quiescence)
    |
    +---> Sprint 5 (branch + rewind)
    |         |
    |         v
    +---> Sprint 6 (suspend/resume + auto-nap) ---> Sprint 6b (Linux suspend, deferred)
    |
    +---> Sprint 7 (shell)
    |
    v
Sprint 8 (SSH auth + remote)
    |
    +---> Sprint 9 (menu bar + auto-start)
    |
    +---> Sprint 10 (MITM SSH + IDE)
    |
    v
Sprint 11 (frontend foundation) --> Sprint 12 (frontend views) --> Sprint 13 (search)
    |
    v
Sprint 14 (polish + enterprise)
    |
    v
Sprint 15 (headless renderer) --> Sprint 16 (renderer UI preview)
```

Sprints 5 and 6 can run in parallel after Sprint 4 (both depend on quiescence).
Sprint 6b (Linux suspend via crosvm crates) is independent, schedulable anytime after Sprint 6.
Sprint 7 (shell) can start after Sprint 4.
Sprints 9, 10 parallel after Sprint 8.
Frontend (11-13) starts after SSH is in place.
Sprint 14 is the core product gate.
Sprints 15-16 (renderer) are post-core -- can start anytime after Sprint 11 (Tauri isolation in place).

## What "done" looks like

- `capsem service start` boots a background daemon managing multiple VMs
- `capsem shell dev` gives interactive PTY, `capsem shell research` opens another
- `ssh dev@localhost:2222` connects VS Code to a VM
- Browser-chrome UI with tabs per VM, side panel for stats/checkpoints/settings
- MCP tools available from Sprint 1 onward, growing with each sprint
- FTS5 search across all session telemetry
- Menu bar shows VM status without opening the app
- Branch a VM environment in <1s (disk reflink), rewind to any snapshot
- Suspend/resume a VM in ~730ms (Apple VZ), auto-nap idle VMs under memory pressure
- Everything Preline, everything typed, everything tested
