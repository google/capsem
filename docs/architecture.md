# Architecture

Capsem is a native macOS application that sandboxes AI agents in lightweight Linux VMs. It uses Apple's Virtualization.framework for hardware-accelerated VM execution on Apple Silicon.

## System Overview

```
+------------------------------------------+
|  Capsem.app (macOS)                      |
|  +------------------------------------+  |
|  | Tauri 2.0 Shell                    |  |
|  | - Svelte 5 WebView (boot UI)      |  |
|  | - VZVirtualMachineView (VM screen) |  |
|  +------------------------------------+  |
|           |  Tauri IPC                    |
|  +------------------------------------+  |
|  | Rust Backend                       |  |
|  | - capsem-core (VM library)         |  |
|  | - Asset resolution                 |  |
|  | - Serial console bridge            |  |
|  | - Auto-updater (native dialog)     |  |
|  +------------------------------------+  |
|           |                               |
|  +------------------------------------+  |
|  | Apple Virtualization.framework     |  |
|  | (objc2-virtualization bindings)    |  |
|  +------------------------------------+  |
|           |                               |
+-----------|-------------------------------+
            v
+------------------------------------------+
| Debian ARM64 Linux VM                    |
| - Custom initramfs with capsem-init      |
| - virtio_console for serial I/O          |
| - ext4 rootfs                            |
+------------------------------------------+
```

## Crate Architecture

The Rust workspace contains two crates:

### capsem-core

The core VM library. Framework-agnostic -- no Tauri dependency.

```
crates/capsem-core/src/
  lib.rs              Public API: VmConfig, VirtualMachine
  vm/
    config.rs         VmConfig builder (CPU, RAM, kernel, initrd, disk, hashes)
    boot.rs           VZLinuxBootLoader setup via objc2-virtualization
    machine.rs        VirtualMachine create/start/stop lifecycle
    serial.rs         Serial console I/O via NSPipe + broadcast channel
```

**Key types:**

- `VmConfig` -- builder pattern for VM configuration. Validates CPU count, RAM size, kernel path. Optionally accepts SHA-256 hashes for boot asset integrity verification.
- `VirtualMachine` -- wraps `VZVirtualMachine`. Provides `create()` which returns the VM, a `broadcast::Receiver<String>` for serial output, and a raw file descriptor for serial input.

### capsem-app

The Tauri application binary. Handles GUI, CLI mode, asset resolution, and auto-updates.

```
crates/capsem-app/src/
  main.rs             Entry point, asset resolution, VM boot, updater
  commands.rs         Tauri IPC commands (vm_status, serial_input)
  state.rs            AppState with Mutex-wrapped VM handle + serial fd
```

**Dual-mode operation:**

- **GUI mode** (no arguments): Launches Tauri window, boots VM, replaces the Svelte WebView with `VZVirtualMachineView` for direct framebuffer display.
- **CLI mode** (with arguments): Boots VM headlessly, sends command via serial console with sentinel markers, captures output between markers, prints to stdout, exits.

## Asset Resolution

The app needs four files: `vmlinuz`, `initrd.img`, `rootfs.img`, `SHA256SUMS`. The `resolve_assets_dir()` function searches these locations in order:

1. `CAPSEM_ASSETS_DIR` environment variable (development override)
2. `Contents/Resources/` inside the .app bundle (production)
3. `./assets/` relative to CWD (workspace root, for `cargo run`)
4. `../../assets/` relative to CWD (when CWD is `crates/capsem-app/`)

In the release .app bundle, Tauri copies assets into `Capsem.app/Contents/Resources/` during `cargo tauri build`. The binary at `Contents/MacOS/capsem` derives the Resources path from `std::env::current_exe()`.

## Build-Time Integrity

The `build.rs` script reads `assets/SHA256SUMS` and embeds the hashes as compile-time constants via `cargo:rustc-env`. At runtime, `capsem-core` verifies each asset's SHA-256 hash before booting the VM.

```
build.rs reads SHA256SUMS
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
5. Generates `SHA256SUMS` for all artifacts
6. Outputs everything to `assets/`

## GUI Architecture

The GUI uses a two-phase approach:

1. **Boot phase**: Tauri renders the Svelte 5 frontend (status indicator, serial console panel). This is visible briefly during VM startup.
2. **Running phase**: Once the VM boots, the Svelte WebView is replaced with `VZVirtualMachineView`, which provides direct framebuffer access to the Linux VM's console. The user interacts with the VM as if it were a native terminal.

The WebView replacement happens via raw AppKit/NSWindow manipulation using `objc2-app-kit` bindings.

## Serial Console Bridge

In GUI mode, serial output from the VM is forwarded as Tauri events (`serial-output`) to the Svelte frontend for display in the Serial Console panel. The serial channel uses `tokio::sync::broadcast` for multi-consumer delivery.

In CLI mode, serial output is parsed directly on the main thread. Commands are wrapped in sentinel markers (`<<<CAPSEM_START>>>` / `<<<CAPSEM_DONE>>>`) to extract just the command output from the serial stream.

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
  2. frontend           Build Svelte (pnpm build)
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
  src/
    routes/+page.svelte     Main UI: VM status, serial console
    routes/+layout.svelte   App shell
    app.html                HTML entry
    app.css                 TailwindCSS 4
  svelte.config.js          SvelteKit with static adapter
  vite.config.ts            Vite bundler config
```

The frontend uses SvelteKit with `@sveltejs/adapter-static` for Tauri compatibility, Skeleton UI components, and Tauri's JavaScript API for IPC.

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
| `sha2` | Build-time and runtime hash verification |
| `tracing` | Structured logging |

## Future Architecture (Planned)

The current implementation covers Milestone 1 (VM boot + serial console). The planned architecture extends to:

- **Guest agent as PID 1** replacing the shell-based init (Milestone 2)
- **vsock control channel** for structured host-guest communication (Milestone 2)
- **Air-gapped networking** via fake-IP SNI routing over vsock (Milestone 5)
- **Transparent AI API proxy** with key injection and cost tracking (Milestone 6)
- **MCP gateway** with policy enforcement and Seatbelt sandboxing (Milestone 7)
- **Session persistence** with overlay disks and security-scoped bookmarks (Milestone 8)

See [docs/status.md](status.md) for milestone progress and [docs/overall_plan.md](overall_plan.md) for the full roadmap.
