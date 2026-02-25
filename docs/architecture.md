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
+---------------------------------------------+
```

## Crate Architecture

The Rust workspace contains four crates:

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

**Framing:** `[4-byte BE length][MessagePack payload]`, max frame size 8KB.

### capsem-core

The core VM library. Framework-agnostic -- no Tauri dependency.

```
crates/capsem-core/src/
  lib.rs              Public API re-exports
  vm/
    config.rs         VmConfig builder (CPU, RAM, kernel, initrd, disk, hashes)
    boot.rs           VZLinuxBootLoader setup via objc2-virtualization
    machine.rs        VirtualMachine create/start/stop lifecycle
    serial.rs         Serial console I/O via NSPipe + broadcast channel
    vsock.rs          VsockManager, CoalesceBuffer, port constants, re-exports from capsem-proto
  net/
    domain_policy.rs  Allow/block list with wildcard matching
    policy_config.rs  user.toml / corp.toml loader, merge logic, GuestConfig
    sni_parser.rs     TLS ClientHello SNI extraction
    sni_proxy.rs      Host-side SNI proxy (vsock:5002 -> real HTTPS)
    telemetry.rs      Per-session web.db (SQLite) for connection logging
```

**Key types:**

- `VmConfig` -- builder pattern for VM configuration. Validates CPU count, RAM size, kernel path. Optionally accepts BLAKE3 hashes for boot asset integrity verification.
- `VirtualMachine` -- wraps `VZVirtualMachine`. Provides `create()` which returns the VM, a `broadcast::Receiver<Vec<u8>>` for serial output, and a raw file descriptor for serial input.
- `VsockManager` -- ObjC bridge that registers vsock listeners on the VM's socket device and delivers accepted connections via an async channel.
- `CoalesceBuffer` -- batches small output chunks (10ms/64KB) to prevent IPC saturation.
- `DomainPolicy` -- evaluates domains against allow/block lists with wildcard support.
- `GuestConfig` -- `[guest]` section from user.toml with env var overrides.

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
3. Wait for `HostToGuest::BootConfig { epoch_secs, env_vars }`
4. Set system clock via `clock_settime(CLOCK_REALTIME)`
5. Open PTY pair, fork bash with env vars from BootConfig
6. Send `GuestToHost::BootReady`
7. Enter bridge loop: master PTY <-> vsock terminal, control loop in background thread

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
Guest                         Host
  |                              |
  |--- Ready { version } ------>|   "I'm alive, send me config"
  |                              |
  |<-- BootConfig { clock, env } |   "Here's your clock + env vars"
  |                              |
  |--- BootReady -------------->|   "Applied config, bash forked, ready"
  |                              |
  |    (terminal I/O begins)     |
```

**Clock synchronization**: The host sends `epoch_secs` (current Unix time) in `BootConfig`. The guest agent calls `clock_settime(CLOCK_REALTIME)` before forking bash. This ensures TLS cert validation, git timestamps, and other time-dependent tools work correctly.

**Environment injection**: `BootConfig.env_vars` carries environment variables with priority: hardcoded defaults (`TERM`, `HOME`, `PATH`, `LANG`) < `user.toml [guest].env` < CLI `--env` flags.

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

## Future Architecture (Planned)

The current implementation covers Milestones 1-3 (VM boot, serial console, vsock PTY agent, CLI exec) and partial Milestone 5 (air-gapped networking with SNI proxy). The planned architecture extends to:

- **VirtioFS workspace sharing** for host-guest file access (Milestone 4)
- **Active AI audit gateway** with 9-stage event lifecycle, PII scrubbing, and tool call interception (Milestone 6)
- **Hybrid MCP gateway**: local tools in-VM, remote tools via host with credential injection (Milestone 7)
- **Per-session audit databases**, config write-back, and enterprise observability (Milestone 8)

See [docs/status.md](status.md) for milestone progress and [docs/overall_plan.md](overall_plan.md) for the full roadmap.
