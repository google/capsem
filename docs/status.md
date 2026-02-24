# Capsem Status

## Milestone 1: Tauri Scaffold + Boot a Linux VM from Rust

### Done

- Cargo workspace with `capsem-core` and `capsem` (renamed from `capsem-app`) crates
- `VmConfig` builder with validation (cpu, ram, kernel, initrd, disk)
- `VZLinuxBootLoader` setup via `objc2-virtualization`
- `SerialConsole` with NSPipe-backed broadcast channel
- `VirtualMachine` create/start/stop wrapping `VZVirtualMachine`
- `VirtualMachine::create()` returns `(Self, broadcast::Receiver<String>)` for serial output
- `VZVirtioEntropyDeviceConfiguration` added (prevents guest hangs on `/dev/urandom`)
- Tauri 2.0 app with setup hook that boots VM on launch (non-fatal on failure)
- Serial output bridged to frontend via Tauri events (`serial-output`)
- Astro frontend with tab bar, xterm.js terminal (shadow DOM), and status bar
- VM image builder (`images/build.py`) producing kernel, initrd, rootfs
- **CLI command execution mode**: `capsem <command>` boots the VM headlessly, runs the command in the guest, prints output to stdout, and exits
- Serial output verified end-to-end on Apple Silicon (kernel boot messages + command output)
- 42 unit tests passing, 0 clippy warnings
- `entitlements.plist` with `com.apple.security.virtualization`

### How to Run

```sh
# Install frontend deps
cd frontend && pnpm install && cd ..

# Build VM assets (first time)
cd images && python3 build.py && cd ..

# Run tests
cargo test --workspace

# Frontend-only dev (no VM, unsigned binary is fine)
cargo tauri dev

# Build + sign (required for VM on Apple Silicon)
make build && make sign

# CLI mode: run a command in the VM
CAPSEM_ASSETS_DIR=$(pwd)/assets target/debug/capsem 'uname -a'

# GUI mode (no arguments)
CAPSEM_ASSETS_DIR=$(pwd)/assets target/debug/capsem
```

### Known Issues

- Debian arm64 kernel ships `virtio_console` as a module, not built-in. CLI mode uses `break=modules` kernel cmdline to let the initramfs load it before dropping to a shell.
- The Tauri GUI serial console panel has not been verified end-to-end (the kernel cmdline change to `break=modules` may affect GUI mode).

### Not Yet Done (Milestone 1 remaining)

- Verify serial console panel works in the Tauri GUI with the updated kernel cmdline

### Not Started

- Milestone 2: Guest Agent as PID 1 + vsock control channel
- Milestone 3: Immutable Debian base image builder
- Milestone 4: VirtioFS shared directories + PTY over vsock
- Milestone 5: Network boundaries & real-time telemetry
- Milestone 6: Active AI audit gateway
- Milestone 7: Hybrid MCP architecture
- Milestone 8: State, audit, and observability
- Milestone 9: Full Tauri UI
- Milestone 10: Stats dashboard + MCP approval UI
- Milestone 11: Polish, security hardening, multi-session
