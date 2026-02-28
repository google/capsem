# Capsem Status

## Implemented Milestones

### Milestone 1: Tauri Scaffold + Boot a Linux VM from Rust
- VmConfig builder with validation (cpu, ram, kernel, initrd, disk, hashes)
- `VZLinuxBootLoader` and `VirtualMachine` abstractions
- Async Tauri 2.0 app booting the VM headlessly or via GUI

### Milestone 2: Guest Agent as PID 1 + vsock Control Channel
- `capsem-init` (bash) and `capsem-pty-agent` (Rust) booting the VM in <100ms
- Vsock control channel (MessagePack framing) for `Exec`, `Resize`, `SetEnv`, `FileWrite`
- Clock synchronization and env var injection at boot

### Milestone 3: Immutable Debian Base Image Builder
- `build.py` pipeline for generating `vmlinuz`, `initrd`, and `rootfs.img`
- Debian bookworm-slim ARM64 base with Node.js, Python, uv, and pre-installed AI CLIs
- Fast hashing using BLAKE3 (B3SUMS)
- Ext4 rootfs with `noatime,nodiratime,noload`

### Milestone 4: PTY over vsock & Workspace (Partially Complete)
- **Done:** PTY allocation in guest, vsock bridging to host, xterm.js terminal UI, `stty size` resize sync.
- **Done:** High-performance async terminal polling and batched input.
- **Pending:** VirtioFS shared directories (workspace, config, caches). Currently using a large, ephemeral ext4 scratch disk instead.

### Milestone 5: Network Boundaries & Real-Time Telemetry
- Air-gapped VM: no real NICs, `dummy0` interface, fake DNS (all to `10.0.0.1`).
- `iptables` REDIRECT routing port 443 to `capsem-net-proxy` on guest.
- **Tokio-based Async `net-proxy`**: Bridges TCP to vsock with TCP_NODELAY and high concurrency.
- Host-side MITM proxy on vsock:5002 intercepting TLS and applying HTTP method/path/domain policies.

### Milestone 6: The Active AI Audit Gateway
- Host-side Axum gateway intercepting AI traffic (`api.anthropic.com`, `api.openai.com`, etc.).
- Real-time SSE stream parsing for Anthropic, OpenAI, and Google Gemini.
- macOS Keychain key injection (no keys inside VM).
- Cost estimation via bundled pydantic pricing models.

### Milestone 8: State, Audit, and Observability
- Unified `capsem-logger` crate with a dedicated writer thread.
- Single `session.db` (SQLite) per VM tracking `net_events`, `model_calls`, `tool_calls`, and `tool_responses`.
- SQL-driven statistics aggregation for the UI dashboard.

### Milestone 9: Full Tauri UI - Session Manager + Settings
- Svelte 5 + Tailwind v4 + DaisyUI v5 frontend.
- Typed settings system (User + Corp policy overrides) translating to `GuestConfig` and `NetworkPolicy`.
- Interactive session dashboard, settings editor, and network events table.

### Milestone 12: Kernel Hardening -- Custom Minimal Kernel
- Custom Linux 6.6.127 kernel with `CONFIG_MODULES=n`, `CONFIG_INET=n` (at kernel level), and stripped drivers.
- Extreme footprint reduction (7MB kernel, 966KB initrd).
- Tuned disk I/O (`scheduler=none`, `VZDiskImageCachingMode::Cached`, 4MB read-ahead) pushing 2M+ IOPS on VirtIO block devices.

## Upcoming Architecture Tasks (Next-Gen Roadmap)

Based on `docs/next-gen.md`, Capsem is evolving from a single-process GUI into a multi-VM platform with a background daemon, MCP server, and hypervisor abstractions.

1. **Phase 1: Hypervisor Abstraction (Linux Readiness)**
   - Extract Apple `Virtualization.framework` code into a clean `AppleVz` backend.
   - Define `Hypervisor`, `VmInstance`, `SerialConsole`, and `VsockProvider` traits.
   - Paves the way for Linux/KVM porting.

2. **Phase 2: Daemon Core + MCP Server**
   - Create `capsem-daemon` crate.
   - Introduce Axum HTTP API for background VM management (`/health`, `/status`, `/stop`).
   - Built-in MCP server so AI agents (like Claude Code) can programmatically `provision_sandbox` and `run_exec`.

3. **Phase 3: Shell & Multi-VM (`capsem shell`)**
   - Interactive PTY via raw mode `stdin`/`stdout`.
   - Ability to attach to daemon-managed VMs.

4. **Phase 4: SSH Gateway & IDE Integration**
   - In-guest `openssh-server` + `capsem-ssh-bridge`.
   - Host-side Unix socket bridge for VS Code / Antigravity Remote SSH.

5. **VirtioFS Workspace Sharing**
   - Fulfill the missing piece of Milestone 4: replacing the ephemeral scratch disk with a live view of the host's project directory.