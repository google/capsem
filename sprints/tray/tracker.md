# System Tray Sprint (S12) -- Standalone `capsem-tray`

macOS menu bar tray as a standalone lightweight binary. Polls the gateway `GET /status` for VM state. Launches capsem-ui on demand. Fully independent from both the UI crate and the gateway crate.

Worktree: worktrees/capsem-tray (branch: s12/menu-bar)
Crate: crates/capsem-tray/

## Architecture

```
capsem-service (daemon)
  |-- spawns capsem-gateway (TCP reverse proxy)
  |-- spawns capsem-tray (menu bar)
       |
       |-- GET http://127.0.0.1:{port}/status
       |-- Authorization: Bearer <token>
       |
       v
  capsem-gateway -> capsem-service (UDS)
```

The tray is a child process of the service. It talks to the service through the gateway over HTTP, same as the browser and UI. No UDS, no capsem-core dependency, no Tauri. Pure Rust with `tray-icon` for native macOS NSStatusItem.

### Tray Lifecycle

The service spawns `capsem-tray` on startup after the gateway is ready (token + port files written). Kills it on shutdown. If the service auto-starts at login (S13), the tray comes up automatically. No separate LaunchAgent needed.

### Token Discovery

1. Read port from `~/.capsem/run/gateway.port`
2. Read token from `~/.capsem/run/gateway.token`
3. All requests: `Authorization: Bearer <token>` to `http://127.0.0.1:{port}`
4. If gateway restarts (new token), tray hot-reloads by re-reading the files

## Crate Structure

```
crates/capsem-tray/
  Cargo.toml
  src/
    main.rs       # Entry point, clap args, event loop, action dispatch
    menu.rs       # Build menu from status response
    gateway.rs    # HTTP client for gateway (reqwest, token reading)
    icons.rs      # Icon loading + state management
  icons/
    tray-default.png    # 22x22 grey template (@1x, @2x)
    tray-active.png     # 22x22 green
    tray-error.png      # 22x22 red
```

### Dependencies

```toml
[package]
name = "capsem-tray"
version.workspace = true
edition = "2021"

[[bin]]
name = "capsem-tray"
path = "src/main.rs"

[dependencies]
tray-icon = "0.21"         # Native NSStatusItem, same lib Tauri uses
muda = "0.16"              # Menu library (tray-icon peer dep)
image = "0.25"             # Icon loading
reqwest = { workspace = true }
tokio = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
anyhow = { workspace = true }
clap = { workspace = true }
```

No capsem-core. No Tauri. No objc2.

## Menu Structure

```
[Tray Icon: green/grey/red]
  |
  +-- "dev -- running"           # Per-VM (named)
  |     +-- Connect              # Opens UI focused on this VM
  |     +-- Suspend / Resume     # Toggle (S7 lands soon)
  |     +-- Fork                 # Snapshot VM to image
  |     +-- Stop
  |     +-- Delete
  |
  +-- "abc123 -- running"        # Per-VM (unnamed, short id)
  |     +-- Connect
  |     +-- Suspend / Resume
  |     +-- Fork
  |     +-- Stop
  |     +-- Delete
  |
  +-- ────────────────
  +-- New Temporary VM           # Provision ephemeral + open UI
  +-- New Long-term VM           # Provision persistent/named + open UI
  +-- Open Capsem                # Launch/focus UI (no specific VM)
  +-- ────────────────
  +-- Quit
```

When gateway is unreachable:

```
[Tray Icon: red]
  |
  +-- Service unavailable
  +-- ────────────────
  +-- Quit
```

## Sub-sprints

### SS1: Scaffold

Status: Done

- [x] Create crate: `crates/capsem-tray/Cargo.toml` with deps (tray-icon 0.22, muda 0.17, image 0.25)
- [x] Add `capsem-tray` to workspace `Cargo.toml` members list
- [x] `main.rs`: clap args (`--port`, `--interval` default 5s), init tracing
- [x] Create `tray_icon::TrayIcon` with placeholder grey icon on main thread
- [x] Main-thread event loop using `MenuEvent::receiver()` + 16ms poll
- [x] Spawn tokio runtime on background thread for async work
- [x] Channel (std::sync::mpsc) from async poller to main-thread menu rebuilder
- [x] Channel (std::sync::mpsc) from main thread to async runtime for actions
- [x] Verify: `cargo build -p capsem-tray` succeeds, clippy clean

### SS2: Gateway Client

Status: Done

- [x] `gateway.rs`: `GatewayClient` struct with `port: u16`, `token: String`, `client: reqwest::Client`
- [x] `discover()` constructor: read `~/.capsem/run/gateway.port` + `gateway.token`
- [x] `status()` -> `GET /status` with Bearer token, returns `StatusResponse`
- [x] `stop_vm(id)` -> `POST /stop/{id}`
- [x] `delete_vm(id)` -> `DELETE /delete/{id}`
- [x] `suspend_vm(id)` -> `POST /suspend/{id}`
- [x] `resume_vm(id)` -> `POST /resume/{id}`
- [x] `fork_vm(id)` -> `POST /fork/{id}`
- [x] `provision_temp()` -> `POST /provision` (ephemeral, default config)
- [x] `provision_named(name)` -> `POST /provision` with name (persistent)
- [x] Token hot-reload: on poll failure, re-discover gateway (re-read port+token files)
- [x] Response types: `StatusResponse`, `VmSummary` with Deserialize
- [ ] Verify: with gateway + service running, `GatewayClient::status()` returns valid response (blocked on gateway landing)

### SS3: Menu Builder

Status: Done

- [x] `menu.rs`: `build_menu(status: &StatusResponse) -> muda::Menu`
- [x] Per-VM submenu: Connect, Suspend/Resume (toggle based on vm.status), Fork, Stop, Delete
- [x] VM display: `"{name} -- {status}"` for named, `"{short_id} -- {status}"` for unnamed
- [x] Global items: "New Temporary VM", "New Long-term VM", "Open Capsem", separator, "Quit"
- [x] `build_unavailable_menu() -> muda::Menu` for when gateway is down
- [x] Menu item IDs encode action + VM id: `"connect:abc123"`, `"stop:abc123"`, `"new-temp"`, `"new-named"`, `"open"`, `"quit"`
- [x] `parse_action(id: &MenuId) -> Option<Action>` for dispatch

### SS4: Polling + Icon State

Status: Done (placeholder icons)

- [x] `icons.rs`: three states: Active (green), Idle (grey), Error (red)
- [x] `load_icon(state: TrayState) -> tray_icon::Icon` with programmatic 22x22 solid-color placeholders
- [x] Background poller: tokio task that calls `gateway.status()` every N seconds with retry
- [x] On each poll: send `PollResult` (Status or Unavailable) over channel to main thread
- [x] Main thread: on channel receive, rebuild menu + update icon
- [x] State transitions: `vm_count > 0` -> green, `vm_count == 0` -> grey, gateway unreachable -> red
- [ ] Verify: start with no VMs (grey), provision one (green), kill gateway (red) (blocked on gateway)

### SS5: Action Dispatch

Status: Done

- [x] Listen for `MenuEvent` on main thread event loop
- [x] Parse menu item ID to extract action + optional VM id via `parse_action()`
- [x] `"connect:{id}"` -> launch UI with `--connect {id}`
- [x] `"suspend:{id}"` / `"resume:{id}"` -> send to async runtime via channel
- [x] `"fork:{id}"` -> send to async runtime
- [x] `"stop:{id}"` / `"delete:{id}"` -> send to async runtime
- [x] `"new-temp"` -> provision via gateway, then launch UI connected to new VM id
- [x] `"new-named"` -> deferred to UI (tray can't prompt for name, launches UI with `--new-named`)
- [x] `"open"` -> launch/focus Capsem UI
- [x] `"quit"` -> `std::process::exit(0)`
- [x] Actions dispatched in async worker drain loop (immediate on next poll cycle)
- [ ] Verify: click Stop on a VM in tray, VM stops, menu updates (blocked on gateway)

### SS6: Icon Assets

Status: Done

- [x] Pre-rendered from project SVG (icon.svg) via rsvg-convert
- [x] `tray-idle.png` / `@2x` -- grey (#808080)
- [x] `tray-active.png` / `@2x` -- purple (#7C3AED)
- [x] `tray-error.png` / `@2x` -- red (#EF4444)
- [x] 22x22 @1x and 44x44 @2x Retina variants
- [x] icons.rs uses `include_bytes!` + `png` crate (no `image`/`resvg` bloat)
- [ ] macOS template image flag (white/black auto-adapts to light/dark mode)
- [ ] Verify: icons render correctly in both light and dark mode

## Acceptance Criteria (Sprint Gate)

- [ ] `cargo build -p capsem-tray` succeeds
- [ ] Tray icon appears in macOS menu bar on launch
- [ ] Polls gateway `/status` every 5s, menu shows VM list
- [ ] Per-VM actions work: Connect, Stop, Delete, Fork
- [ ] "New Temporary VM" provisions and opens UI
- [ ] "New Long-term VM" provisions and opens UI
- [ ] "Open Capsem" launches/focuses UI window
- [ ] Icon: green with VMs, grey without, red when gateway down
- [ ] Token hot-reload works after gateway restart
- [ ] "Quit" exits cleanly

## Depends On

- **capsem-gateway** running (for HTTP access to service)
  - `GET /status` endpoint (SS4 in gateway tracker)
  - `gateway.token` + `gateway.port` files (SS2 in gateway tracker)
  - All proxied service endpoints

## Requirements for Other Teams

### Service team (amend `sprints/next-gen/tracker.md`)

- capsem-service spawns `capsem-tray` as child process on startup, after gateway is ready
- Kills capsem-tray on shutdown

### UI team

- Accept `--connect {vm_id}` CLI arg to open UI focused on a specific VM

## Reference

- Gateway `/status` response: `sprints/gateway/tracker.md` SS4
- Gateway auth flow: `sprints/gateway/tracker.md` SS2
- Service API types: `crates/capsem-service/src/api.rs` (SandboxInfo at line 57)
- Rust patterns: `skills/dev-rust-patterns/SKILL.md`
- Testing policy: `skills/dev-testing/SKILL.md`
