# Architecture

Capsem is a native macOS application that sandboxes AI agents in lightweight Linux VMs. It uses Apple's Virtualization.framework for hardware-accelerated VM execution on Apple Silicon.

## System Overview

```
+---------------------------------------------+
|  Capsem.app (macOS)                         |
|  +---------------------------------------+  |
|  | Tauri 2.0 Shell                       |  |
|  | - Astro WebView (xterm.js terminal)  |  |
|  | - VZVirtualMachineView (VM screen)    |  |
|  +---------------------------------------+  |
|           |  Tauri IPC                       |
|  +---------------------------------------+  |
|  | Rust Backend                          |  |
|  | - capsem-app  (GUI, CLI, IPC)        |  |
|  | - capsem-core (VM library)           |  |
|  | - capsem-proto (protocol types)      |  |
|  | - SNI proxy + domain policy          |  |
|  | - Auto-updater (native dialog)       |  |
|  +---------------------------------------+  |
|           |                                  |
|  +---------------------------------------+  |
|  | Apple Virtualization.framework        |  |
|  | (objc2-virtualization bindings)       |  |
|  +---------------------------------------+  |
|           | serial (boot logs)               |
|           | vsock:5000 (control)             |
|           | vsock:5001 (terminal PTY I/O)    |
|           | vsock:5002 (SNI proxy)           |
+-----------|----------------------------------+
            v
+---------------------------------------------+
| Debian ARM64 Linux VM                       |
| - capsem-init (PID 1): mounts, network,    |
|   launches capsem-pty-agent                  |
| - capsem-pty-agent: PTY <-> vsock bridge,   |
|   boot handshake (Ready/BootConfig/BootReady)|
| - capsem-net-proxy: TCP:10443 -> vsock:5002 |
| - Air-gapped networking (dummy0 + fake DNS  |
|   + iptables REDIRECT for SNI proxy)         |
| - Read-only ext4 rootfs + tmpfs overlays    |
| - Ephemeral ext4 scratch disk (/root, ~8GB) |
+---------------------------------------------+
```

## Crate Architecture

The Rust workspace contains five crates:

### capsem-proto

Shared protocol types for host/guest communication. No platform-specific deps -- cross-compiles for both macOS host and aarch64-linux-musl guest.

```
crates/capsem-proto/src/
  lib.rs              HostToGuest, GuestToHost enums, encode/decode helpers
```

**Key types:**

- `HostToGuest` -- commands from host to guest: `BootConfig`, `Resize`, `Exec`, `Ping`, plus reserved variants (`FileWrite`, `FileRead`, `FileDelete`, `Shutdown`).
- `GuestToHost` -- messages from guest to host: `Ready`, `BootReady`, `ExecDone`, `Pong`, plus reserved variants (`FileCreated`, `FileModified`, `FileDeleted`, `FileContent`).

**Security invariant (RFC T14):** The host only deserializes `GuestToHost`. The guest only deserializes `HostToGuest`. Disjoint serde tags make cross-type decoding fail at the type level.

**Framing:** `[4-byte BE length][MessagePack payload]`, max frame size 256KB.

### capsem-core

The core VM library. Framework-agnostic -- no Tauri dependency.

```
crates/capsem-core/src/
  lib.rs              Public API re-exports
  vm/
    config.rs         VmConfig builder (CPU, RAM, kernel, initrd, rootfs disk, scratch disk, hashes)
    boot.rs           VZLinuxBootLoader setup via objc2-virtualization
    machine.rs        VirtualMachine create/start/stop lifecycle
    serial.rs         Serial console I/O via NSPipe + broadcast channel
    vsock.rs          VsockManager, CoalesceBuffer, port constants, re-exports from capsem-proto
  net/
    domain_policy.rs  Allow/block list with wildcard matching
    policy_config.rs  Settings engine: typed registry, user/corp merge, translation to policy objects
    mitm_proxy.rs     MITM proxy: TLS termination, HTTP inspection, upstream bridging, AI call auditing
    cert_authority.rs CA loader + on-demand domain cert minting with RwLock cache
    http_policy.rs    Method+path policy engine (extends domain-level policy)
    sni_parser.rs     TLS ClientHello SNI extraction
    policy.rs         NetworkPolicy aggregate (domain + HTTP rules)
  gateway/
    mod.rs            GatewayConfig (holds Arc<DbWriter>)
    server.rs         Axum router: proxy handler, key injection, SSE streaming, audit logging
    provider.rs       Provider trait + route_provider (Anthropic, OpenAI, Google)
    anthropic.rs      Anthropic provider + SSE stream parser
    openai.rs         OpenAI provider + SSE stream parser
    google.rs         Google Gemini provider + SSE stream parser
    events.rs         StreamEvent types, collect_summary for SSE audit
    request_parser.rs Structured request body parsing (model, tools, system prompt)
    ai_body.rs        AiResponseBody: streaming body wrapper with parser + stats
    sse.rs            SSE line parser
```

**Key types:**

- `VmConfig` -- builder pattern for VM configuration. Validates CPU count, RAM size, kernel path, scratch disk path. Optionally accepts BLAKE3 hashes for boot asset integrity verification. Supports two block devices: rootfs (read-only, identifier `rootfs`) and scratch disk (read-write, identifier `scratch`).
- `VirtualMachine` -- wraps `VZVirtualMachine`. Provides `create()` which returns the VM, a `broadcast::Receiver<Vec<u8>>` for serial output, and a raw file descriptor for serial input.
- `VsockManager` -- ObjC bridge that registers vsock listeners on the VM's socket device and delivers accepted connections via an async channel.
- `CoalesceBuffer` -- batches small output chunks (10ms/64KB) to prevent IPC saturation.
- `DomainPolicy` -- evaluates domains against allow/block lists with wildcard support.
- `GuestConfig` -- guest environment variables extracted from settings.
- `ResolvedSetting` -- a fully resolved setting with effective value, source, and metadata.

### capsem-agent

Guest-side binaries, cross-compiled for `aarch64-unknown-linux-musl`.

```
crates/capsem-agent/src/
  main.rs             capsem-pty-agent: PTY <-> vsock bridge with boot handshake
  net_proxy.rs        capsem-net-proxy: TCP:10443 -> vsock:5002 relay
  vsock_io.rs         Shared vsock connect + fd I/O helpers
```

**capsem-pty-agent boot sequence:**

1. Connect vsock control (port 5000) and terminal (port 5001)
2. Send `GuestToHost::Ready { version }`
3. Receive `HostToGuest::BootConfig { epoch_secs }` -- clock sync
4. Receive individual `SetEnv` messages (validated: no NUL, no blocked vars, capped at 128)
5. Receive `FileWrite` messages (validated: no path traversal, capped at 64 files / 10MB total)
6. Receive `BootConfigDone` -- end of boot config
7. Set system clock, apply env vars, open PTY, fork bash
8. Send `GuestToHost::BootReady`
9. Enter bridge loop: master PTY <-> vsock terminal, control loop in background thread

### capsem-app

The Tauri application binary. Handles GUI, CLI mode, asset resolution, and auto-updates.

```
crates/capsem-app/src/
  main.rs             Entry point, asset resolution, VM boot, boot handshake, updater
  commands.rs         Tauri IPC commands (vm_status, serial_input, terminal_resize, net_events)
  state.rs            AppState with per-VM instance state (serial + vsock fds, network state)
```

**Dual-mode operation:**

- **GUI mode** (no arguments): Launches Tauri window, boots VM, performs vsock boot handshake (Ready -> BootConfig -> BootReady), then replaces WebView with `VZVirtualMachineView`.
- **CLI mode** (with arguments): Boots VM headlessly, performs boot handshake, sends `Exec` command via vsock control channel, captures output from vsock terminal, propagates exit code. Supports `--env KEY=VALUE` flags.

## Asset Resolution

The app needs four files: `vmlinuz`, `initrd.img`, `rootfs.img`, `B3SUMS`. The `resolve_assets_dir()` function searches these locations in order:

1. `CAPSEM_ASSETS_DIR` environment variable (development override)
2. `Contents/Resources/` inside the .app bundle (production)
3. `./assets/` relative to CWD (workspace root, for `cargo run`)
4. `../../assets/` relative to CWD (when CWD is `crates/capsem-app/`)

In the release .app bundle, Tauri copies assets into `Capsem.app/Contents/Resources/` during `cargo tauri build`. The binary at `Contents/MacOS/capsem` derives the Resources path from `std::env::current_exe()`.

## Build-Time Integrity

The `build.rs` script reads `assets/B3SUMS` and embeds the hashes as compile-time constants via `cargo:rustc-env`. At runtime, `capsem-core` verifies each asset's BLAKE3 hash before booting the VM.

```
build.rs reads B3SUMS
  -> cargo:rustc-env=VMLINUZ_HASH=abc123...
  -> cargo:rustc-env=INITRD_HASH=def456...
  -> cargo:rustc-env=ROOTFS_HASH=789abc...

main.rs:
  option_env!("VMLINUZ_HASH") -> passed to VmConfig builder
  capsem-core verifies hash before loading kernel
```

## VM Image Pipeline

The `images/` directory builds the VM assets using Podman (or Docker):

```
images/
  build.py            Orchestrator: runs container builds, extracts artifacts
  Dockerfile.kernel   Multi-stage build: installs Debian ARM64 kernel,
                      creates custom initramfs with capsem-init
  capsem-init         Custom /init script for the initramfs
  capsem-bashrc       Shell environment for the VM
  modules.txt         Kernel modules to include in initramfs
  hooks/capsem        initramfs-tools hook for module inclusion
```

**Build flow:**

1. `build.py` builds a container from `Dockerfile.kernel` on Debian bookworm ARM64
2. Installs `linux-image-arm64`, extracts the kernel as `vmlinuz`
3. Creates a custom initramfs with `capsem-init` as the init process
4. Creates a 64MB ext4 `rootfs.img` (formatted inside a container since macOS lacks `mkfs.ext4`)
5. Generates `B3SUMS` for all artifacts
6. Outputs everything to `assets/`

## GUI Architecture

The GUI uses a two-phase approach:

1. **Boot phase**: Tauri renders the Astro frontend (tab bar, xterm.js terminal in a shadow DOM web component, status bar). This is visible briefly during VM startup.
2. **Running phase**: Once the VM boots, the WebView is replaced with `VZVirtualMachineView`, which provides direct framebuffer access to the Linux VM's console. The user interacts with the VM as if it were a native terminal.

The WebView replacement happens via raw AppKit/NSWindow manipulation using `objc2-app-kit` bindings.

## Communication Channels

### Serial console (boot logs only)

The serial console (`/dev/hvc0`) carries kernel boot messages. In GUI mode, serial output is forwarded as Tauri events (`serial-output`) to xterm.js via `tokio::sync::broadcast`. Serial forwarding is aborted once the vsock boot handshake completes.

### Vsock (primary terminal + control)

All post-boot communication uses virtio-vsock:

| Port | Direction | Purpose |
|------|-----------|---------|
| 5000 | Bidirectional | Control messages (BootConfig, Resize, Exec, Ping/Pong) |
| 5001 | Bidirectional | Raw PTY byte streaming (terminal I/O) |
| 5002 | Guest -> Host | SNI proxy (HTTPS connections from guest) |

**Boot handshake** (vsock:5000):

```
Guest                                Host
  |                                    |
  |--- Ready { version } ------------>|
  |                                    |
  |<-- BootConfig { epoch_secs } -----|  clock sync
  |<-- SetEnv { key, value } ---------|  (repeated, validated)
  |<-- FileWrite { path, data } ------|  (repeated, validated)
  |<-- BootConfigDone ----------------|  end of config
  |                                    |
  |--- BootReady -------------------->|  config applied, terminal ready
  |                                    |
  |    (terminal I/O begins)           |
```

**Clock synchronization**: The host sends `epoch_secs` (current Unix time) in `BootConfig`. The guest agent calls `clock_settime(CLOCK_REALTIME)` before forking bash. This ensures TLS cert validation, git timestamps, and other time-dependent tools work correctly.

**Environment injection**: Individual `SetEnv` messages carry environment variables with priority: hardcoded defaults (`TERM`, `HOME`, `PATH`, `LANG`) < `user.toml [guest].env` < CLI `--env` flags. Both host and guest validate env vars: keys containing `=` or NUL bytes are rejected, blocked variables (LD_PRELOAD, IFS, BASH_ENV, etc.) are dropped, and the total count is capped at 128. File writes are validated against path traversal (`..`) and capped at 64 files / 10MB total.

**Terminal I/O** (vsock:5001): Frontend xterm.js `onData` -> Tauri `serial_input` command -> vsock fd -> guest PTY. Reverse: guest PTY -> vsock -> `CoalesceBuffer` (10ms/64KB) -> Tauri event -> xterm.js `write`.

**CLI exec**: Host sends `HostToGuest::Exec { id, command }`, agent injects command into PTY with sentinel markers, detects exit code via sentinel in PTY output, sends `GuestToHost::ExecDone { id, exit_code }`.

### SNI proxy (vsock:5002)

Guest-side `capsem-net-proxy` listens on TCP `127.0.0.1:10443`. iptables REDIRECT captures all port 443 traffic to this listener. The proxy bridges each connection to the host via vsock:5002. The host reads the TLS ClientHello, extracts the SNI hostname, checks it against the domain policy (allow/block lists from `user.toml` + `corp.toml`), and either bridges to the real server or rejects the connection. All decisions are logged to per-session `web.db`.

## Auto-Update

The app uses Tauri's updater plugin with minisign signature verification:

1. On launch (before VM boot), the app checks GitHub Releases for a new version
2. If available, a native macOS dialog prompts the user (not a WebView dialog, since the WebView gets replaced)
3. On accept, the update is downloaded, verified against the embedded public key, and installed
4. The app restarts automatically

The updater public key is embedded in `tauri.conf.json`. The private key is used during CI builds (stored as a GitHub Actions secret).

## Release Pipeline

```
make release-sign
  1. assets-check      Verify vmlinuz exists
  2. frontend           Build Astro (pnpm build)
  3. cargo tauri build  Compile Rust, bundle .app with assets in Resources/
  4. codesign            Sign .app with virtualization entitlement
```

CI (`.github/workflows/release.yaml`) additionally:
- Builds VM assets from scratch on Ubuntu (ARM64 via QEMU)
- Enables updater artifact signing via `TAURI_CONFIG` override
- Signs with Developer ID for notarization
- Generates SLSA provenance attestation and SBOM

## Frontend Stack

```
frontend/
  astro.config.mjs                  Astro config (static output)
  src/
    pages/index.astro               Single page: tab bar + terminal + status bar
    components/capsem-terminal.ts   Shadow DOM web component wrapping xterm.js
    styles/global.css               Plain CSS variables (no framework)
```

The frontend uses Astro with static output for Tauri compatibility. The terminal runs inside a closed shadow DOM web component (`capsem-terminal`) with xterm.js + WebGL addon for rendering. Dependencies: `@tauri-apps/api`, `@xterm/xterm`, `@xterm/addon-fit`, `@xterm/addon-webgl`, `astro`.

## Key Dependencies

| Crate | Role |
|-------|------|
| `objc2-virtualization` | Apple Virtualization.framework bindings |
| `objc2` + `objc2-foundation` + `objc2-app-kit` | Objective-C interop, NSWindow manipulation |
| `tauri` v2 | App shell, IPC, bundling |
| `tauri-plugin-updater` | Auto-update with signature verification |
| `tauri-plugin-dialog` | Native macOS dialogs for update prompts |
| `tauri-plugin-process` | App restart after update |
| `tokio` | Async runtime |
| `rmp-serde` | MessagePack serialization for vsock control messages |
| `serde` | Serialization framework |
| `blake3` | Build-time and runtime hash verification |
| `tracing` | Structured logging |
| `nix` | Unix syscalls for guest agent (PTY, signals, fork) |
| `rusqlite` | Per-session web.db for network telemetry |
| `toml` | Policy config parsing (user.toml / corp.toml) |

## Execution Logging

Every VM boot sequence is instrumented with `tracing` spans. The subscriber uses `FmtSpan::CLOSE` so each span logs its duration when it completes. This provides a complete boot performance profile without manual timing code.

**Span hierarchy:**

- `boot_vm` (info-level, always visible)
  - `config_build` -- VmConfig validation and hash verification
    - `verify_hash{path=...}` -- per-asset BLAKE3 verification
  - `vm_create` -- VZVirtualMachine construction
    - `create_boot_loader` -- VZLinuxBootLoader setup
    - `create_serial_port` -- NSPipe serial console setup
    - `vz_configure` -- ObjC config (CPU, RAM, devices)
    - `vz_validate` -- VZ config validation
    - `vz_init` -- VZVirtualMachine instantiation
  - `vm_start` -- VM start + runloop spin

**Usage:** `RUST_LOG=capsem=debug` for full breakdown, `RUST_LOG=capsem=info` for top-level boot time only.

## Disk Architecture

The VM uses two virtio block devices with stable identifiers:

| Device | Identifier | Guest Path | Mode | Purpose |
|--------|-----------|------------|------|---------|
| rootfs | `rootfs` | `/dev/vda` | Read-only | Immutable Debian base image |
| scratch | `scratch` | `/dev/vdb` | Read-write | Ephemeral `/root` workspace |

**Scratch disk lifecycle**: Host creates a sparse file (`~/.capsem/sessions/<vm_id>/scratch.img`), guest formats it at boot (`mke2fs -t ext4 -O ^has_journal`), mounts at `/root`. Deleted on VM stop. No journal for lower I/O overhead on ephemeral data.

**Session directory** (`~/.capsem/sessions/<vm_id>/`):

```
scratch.img      # ephemeral scratch disk (deleted on VM stop)
web.db           # network telemetry (retained across sessions)
session.json     # metadata: vm_id, status, created_at, config snapshot
```

**Stale session cleanup**: On app startup, leftover `scratch.img` files are deleted and orphaned "running" sessions are marked "crashed".

### Future: Custom Disks (Forking)

The current scratch disk is ephemeral -- wiped on every boot. A future release will add **persistent custom disks** that users can configure, save, and reuse:

- **Fork workflow**: Boot VM with a special `--setup` / config mode flag. User installs packages, configures tools, customizes environment. On exit, the scratch disk is NOT deleted -- instead it's saved as a named custom disk image (e.g., `~/.capsem/disks/my-ml-env.img`).
- **Boot from custom disk**: User selects a saved disk image when creating a session. The image is attached as the scratch device instead of a fresh sparse file. Guest skips formatting (detects existing ext4 via superblock check) and mounts directly.
- **Disk metadata**: Tracked in a central database (likely `~/.capsem/capsem.db`) rather than per-session JSON. Schema includes: disk name, creation date, size, base rootfs version, last-used timestamp, status (active/paused/stopped/archived), parent disk (for fork lineage), and associated session IDs.
- **Pause vs stop semantics**: A paused session keeps its disk intact and the VM state can be restored (once VZ checkpointing is wired up). A stopped session can optionally preserve or discard its disk. Disk metadata tracks which state each disk is in, so the UI can show "3 paused sessions, 1 running" etc.

This replaces the per-session `session.json` approach with a proper relational model in `capsem.db` where disks, sessions, and VM lifecycle states are first-class entities.

## Settings Architecture

Capsem uses a generic typed settings system for all configuration. Each setting has an ID, name, description, type, category, default value, and optional `enabled_by` pointer to a parent toggle.

### Setting Registry

Settings are defined in a compile-time registry (`setting_definitions()` in `policy_config.rs`). Categories:

| Category | Example Settings |
|----------|-----------------|
| AI Providers | `ai.anthropic.allow`, `ai.anthropic.api_key`, `ai.openai.allow`, ... |
| Package Registries | `registry.github.allow`, `registry.npm.allow`, `registry.pypi.allow`, ... |
| Network | `network.default_action`, `network.log_bodies`, `network.max_body_capture` |
| Session | `session.retention_days` |
| Appearance | `appearance.dark_mode`, `appearance.font_size` |
| VM | `vm.scratch_disk_size_gb` |
| Guest Environment | `guest.env.*` (dynamic, prefix-based) |

### Setting Types

Each setting has a `SettingType` that drives UI rendering: `Text`, `Number`, `Password`, `Url`, `Email`, `ApiKey`, `Bool`.

### TOML Format

Settings files store only overrides. A setting not listed uses its registry default:

```toml
[settings]
"registry.github.allow" = { value = true, modified = "2026-02-24T10:30:00Z" }
"network.log_bodies" = { value = true, modified = "2026-02-24T10:30:00Z" }
"guest.env.EDITOR" = { value = "vim", modified = "2026-02-24T10:30:00Z" }
```

### Merge Semantics

Resolution order: **corp > user > default**. For each setting ID, the corp file wins if present, then user, then the registry default. This is per-key, not per-category.

### Setting Metadata

Network toggle settings carry structured metadata:

- `domains` -- domain patterns (e.g., `["github.com", "*.github.com"]`) that are allowed/blocked when the toggle is on/off.
- `rules` -- HTTP method permissions per domain (e.g., GET+POST allowed, DELETE denied).
- `choices` -- valid values for text choice settings (e.g., `["allow", "deny"]`).
- `min`/`max` -- bounds for number settings.

### Translation Layer

The settings engine translates resolved settings into domain-specific policy objects:

```
ResolvedSettings --> settings_to_domain_policy() --> DomainPolicy
                 --> settings_to_http_policy()   --> HttpPolicy
                 --> settings_to_guest_config()  --> GuestConfig
                 --> settings_to_vm_settings()   --> VmSettings
```

Enabled toggles with domain metadata add to the allow-list. Disabled toggles with domain metadata add to the block-list. This means toggling `ai.anthropic.allow` from false to true moves `api.anthropic.com` from blocked to allowed.

### enabled_by

A setting can declare `enabled_by: Some("parent.id")` to indicate it depends on a parent toggle. When the parent is off, the child is computed as `enabled: false` (greyed out in the UI). Only one level of nesting is supported.

Example: `ai.anthropic.api_key` has `enabled_by: Some("ai.anthropic.allow")`. The API key field is disabled when the Anthropic toggle is off.

## Future Architecture (Planned)

The current implementation covers Milestones 1-4 (VM boot, serial console, vsock PTY agent, CLI exec, MITM proxy, scratch disk). The planned architecture extends to:

- **Custom disk images** with fork/save workflow for persistent environments
- **Disk + session metadata database** (`capsem.db`) with pause/stop/archive states
- **VM pause/resume** using VZ framework's native pause support
- **VM checkpointing** (macOS 14+) with `saveMachineStateTo` / `restoreMachineStateFrom`
- **VirtioFS workspace sharing** for host-guest file access
- **Active AI audit gateway** with 9-stage event lifecycle, PII scrubbing, and tool call interception
- **Hybrid MCP gateway**: local tools in-VM, remote tools via host with credential injection
- **Per-session audit databases**, config write-back, and enterprise observability

See [docs/status.md](status.md) for milestone progress and [docs/overall_plan.md](overall_plan.md) for the full roadmap.

## Development Guidelines

When extending the Rust backend or guest agents, adhere to the following performance and concurrency guidelines to ensure system stability under heavy load:

1. **Never Block the Async Executor or Tauri IPC Thread Pool:**
   - The Tauri backend and Axum gateway run on asynchronous executors (`tokio`). Performing synchronous, long-running operations (like heavy CPU bound tasks or blocking disk I/O, including writing to a vsock buffer that might be full) directly inside an `async` function or a synchronous Tauri command will stall the worker thread. For Tauri commands, this exhausts the IPC thread pool, causing the UI to freeze and queue up inputs (lag/barfing).
   - **Rule:** Always define Tauri commands that do I/O as `async fn` and wrap synchronous disk or vsock operations (e.g., SQLite writes, heavy file reads, `write_all` to a file descriptor) inside `tokio::task::spawn_blocking` to offload them to Tokio's dedicated background thread pool.

2. **Avoid Thread Pool Exhaustion and Latency in Hot Loops:**
   - While `spawn_blocking` is essential for disk I/O, calling it excessively inside high-frequency hot loops (e.g., per-keystroke from the terminal, or per-byte/chunk in a stream parser) will flood the Tokio thread pool. This causes severe context-switching overhead, CPU spikes, and lag.
   - **Rule:** For high-frequency events, do NOT spawn a new blocking task per event. Instead, use an `std::sync::mpsc::channel` to send the events instantly to a *single* dedicated background thread.
   - **Implementation (Terminal Input):** The `terminal_input_tx` in `AppState` uses a dedicated thread that survives the entire application lifecycle. It coalesces rapid sequential keystrokes using `try_recv` and reuses a single `File` handle (avoiding `dup/close` syscalls per char).
   - **Rule (Guest Agent Buffering):** When scanning for sentinels in a stream (like the exit-code sentinel in `capsem-pty-agent`), never use fixed-size buffering that forces a lag. Only buffer the minimal amount of data that *actually matches* a prefix of the target sentinel, and flush everything else immediately to maintain real-time interactive performance.

3. **Prevent Bidirectional I/O Deadlocks:**
   - When bridging two blocking file descriptors bidirectionally (e.g., bridging a TCP socket to a vsock, or bridging the PTY to the vsock), doing both reads in a single thread using `poll(2)` is vulnerable to deadlocks. If both outgoing buffers fill up simultaneously, the single thread blocks on writing and stops reading, creating a mutual lockup.
   - **Rule:** Always spawn a dedicated background thread to handle at least one direction of the bidirectional data flow.

4. **Optimize Payload Parsing:**
   - The LLM Gateway handles massive HTTP payloads (megabytes of tool calls or images). Parsing these entirely into dynamic memory structures (like `serde_json::Value`) is highly inefficient and risks memory exhaustion.
   - **Rule:** When extracting specific fields from a large JSON payload, define a targeted struct and use `serde::Deserialize`. Serde will perform a structural parse, skipping and discarding unused data without allocating memory for it. In stream parsers, state updates must be lock-free or use fast memory-only `Mutex` locks without triggering any blocking I/O per chunk.
