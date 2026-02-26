# Capsem Next-Gen Platform Architecture

Terminal, IDE, and Agent Integration

## Context

Capsem is a macOS app that sandboxes AI agents in Linux VMs. Currently it operates as a Tauri GUI or a fire-and-forget exec CLI. The goal is to evolve Capsem into a **platform** with:

- A **hypervisor abstraction** enabling future Linux/KVM support
- A **multi-VM daemon** with HTTP management API
- **Interactive terminal** access (`capsem shell`)
- **SSH gateway** for VS Code / Google Antigravity (VS Code fork) remote development
- **MCP server** so AI agents can programmatically create and control sandboxes
- The **Tauri GUI becoming a daemon client** (UI can close/reopen without killing VMs)

## Command Structure

```
capsem                          -> GUI (Tauri, client of daemon)
capsem ui                       -> GUI (explicit alias)
capsem shell [--env K=V]...     -> interactive PTY session
capsem start [--env K=V]... [--name <id>]  -> start background VM
capsem stop [<id>]              -> gracefully stop VM
capsem status                   -> list running VMs (queries daemon HTTP API)
capsem ssh-config [<id>]        -> print SSH config snippet
capsem [--env K=V]... <command> -> exec mode (backward compat)
```

## Debugging & Testing Workflows This Enables

```sh
# 1. Interactive investigation after failures
capsem shell                          # drop into VM, poke around
capsem-doctor -k sandbox -x           # run individual diagnostics

# 2. Keep VM alive while iterating
capsem start                          # boot once
ssh capsem-default "curl https://github.com"  # test repeatedly
capsem stop                           # done

# 3. Multi-terminal debugging
capsem start                          # one VM
capsem shell                          # terminal 1: interactive
ssh capsem-default "tail -f /var/log/..."  # terminal 2: logs

# 4. VS Code / Antigravity remote development
capsem start && code --remote ssh-remote+capsem-default /workspace

# 5. AI agent control (MCP)
# Claude Code calls provision_sandbox(), run_exec(), read_file() via MCP

# 6. HTTP management
curl http://localhost:9800/status      # check all VMs
curl http://localhost:9800/logs/dev    # stream VM logs
```

---

## Phase 1: Hypervisor Abstraction (Linux Readiness)

**Goal**: Define `Hypervisor` and `VmInstance` traits. Isolate all Apple VZ code behind an `AppleVz` backend. Zero functional changes -- pure refactor.

### Current VZ touchpoints

VZ-specific code lives in 4 files (machine.rs, boot.rs, serial.rs, vsock.rs). Already platform-agnostic: config.rs, host_state.rs, all of net/*.

| File | VZ coupling | What to abstract |
|------|-------------|------------------|
| `vm/machine.rs` | Heavy -- `VZVirtualMachine`, `VZVirtualMachineConfiguration`, ObjC | `Hypervisor` trait: start/stop/state |
| `vm/boot.rs` | Medium -- `VZLinuxBootLoader`, `NSURL` | Move into AppleVz backend |
| `vm/serial.rs` | Heavy -- `VZFileHandleSerialPortAttachment`, NSPipe | `SerialConsole` trait: reader + input fd |
| `vm/vsock.rs` | Partial -- VZ listener delegation is ObjC, but `CoalesceBuffer`/`VsockConnection` are generic | `VsockProvider` trait: accept/connect |

### Trait design

New file: `crates/capsem-core/src/hypervisor/mod.rs`

**Mandate**: No `objc2` or `Virtualization` symbols may appear outside of `src/hypervisor/apple_vz/`.

```rust
/// Factory trait: creates VmInstance from config.
pub trait Hypervisor: Send + Sync {
    fn boot(&self, config: VmConfig) -> Result<Box<dyn VmInstance>>;
}

/// A running VM instance (platform-agnostic handle).
pub trait VmInstance: Send + Sync {
    fn stop(&self) -> Result<()>;
    fn cid(&self) -> u32;  // vsock CID
    fn state(&self) -> VmState;
    fn vsock_provider(&self) -> &dyn VsockProvider;
    fn serial_console(&self) -> &dyn SerialConsole;
}

pub trait SerialConsole: Send + Sync {
    fn subscribe(&self) -> broadcast::Receiver<Vec<u8>>;
    fn input_fd(&self) -> RawFd;
}

pub trait VsockProvider: Send {
    fn accept(&mut self) -> Option<VsockConnection>;
    fn try_accept(&mut self) -> Result<VsockConnection, TryRecvError>;
}

pub enum VmState { Created, Booting, Running, Stopped, Error }
```

### New file structure

```
crates/capsem-core/src/
  hypervisor/
    mod.rs              -- Hypervisor, SerialConsole, VsockProvider traits
    apple_vz/
      mod.rs            -- AppleVzHypervisor implementation
      machine.rs        -- current vm/machine.rs logic (VZ-specific)
      boot.rs           -- current vm/boot.rs
      serial.rs         -- current vm/serial.rs
      vsock.rs          -- VZ listener delegation from current vm/vsock.rs
  vm/
    config.rs           -- stays (already platform-agnostic)
    vsock.rs            -- CoalesceBuffer + VsockConnection stay (platform-agnostic)
```

### Feature gate

```toml
# capsem-core/Cargo.toml
[features]
default = ["apple-vz"]
apple-vz = ["objc2-virtualization", "objc2", "objc2-foundation", "block2", "dispatch2", "core-foundation-sys"]
```

### Key constraint: `inner_vz()` removal

`machine.rs:143-144` exposes the raw `ObjcVZVirtualMachine`. This must be removed -- all functionality it provides must be available through trait methods. Currently `socket_devices()` returns `NSArray<VZSocketDevice>` -- this leaks VZ types into the app layer and must be replaced with the `VsockProvider` trait.

### Files to modify
- `crates/capsem-core/src/vm/machine.rs` -> move to `hypervisor/apple_vz/machine.rs`
- `crates/capsem-core/src/vm/boot.rs` -> move to `hypervisor/apple_vz/boot.rs`
- `crates/capsem-core/src/vm/serial.rs` -> move to `hypervisor/apple_vz/serial.rs`
- `crates/capsem-core/src/vm/vsock.rs` -> split: VZ parts to `apple_vz/vsock.rs`, keep `CoalesceBuffer`/`VsockConnection` in `vm/vsock.rs`
- `crates/capsem-core/src/lib.rs` -> export `hypervisor` module
- `crates/capsem-app/src/state.rs` -> use `Box<dyn Hypervisor>` instead of `VirtualMachine`
- `crates/capsem-app/src/main.rs` -> use trait methods, remove `inner_vz()` calls

### Verification
- `cargo test --workspace` passes (all existing tests)
- `just check` passes
- `just repack "echo capsem-ok"` boots and runs (no behavioral changes)
- `cfg(not(feature = "apple-vz"))` compiles (no VZ symbols in trait definitions)

---

## Phase 2: Daemon Core + MCP (`capsem-daemon` crate)

**Goal**: A new `capsem-daemon` crate manages VMs as an orchestrator with an axum HTTP API **and an MCP server from day one**. MCP comes early so Claude Code can directly interact with VMs during development -- enabling faster debugging of all subsequent phases.

### Architecture

```
capsem start --name dev
  |
  fork() + setsid()
  |
  Parent: prints "VM started (id=dev, pid=12345)", exits
  |
  Child (capsem-daemon):
    +-- Orchestrator: state machine for multiple VmInstances
    +-- Hypervisor (AppleVz): owns VM, vsock, serial
    +-- MITM proxy: handles HTTPS inspection
    +-- HTTP API (axum): localhost:9800
    |     GET /health, /status, /list, /logs/{vm_id}
    |     POST /stop/{vm_id}
    +-- MCP server: provision_sandbox, run_exec, list_sandboxes, shutdown
    +-- Unix socket: ~/.capsem/sessions/dev/ssh.sock (Phase 4)
    +-- CFRunLoop pumping (VZ requirement on macOS)
```

### New crate: `crates/capsem-daemon/`

```
crates/capsem-daemon/src/
    main.rs          -- process forking, PID management, signal handling
    orchestrator.rs  -- state machine for multiple VmInstances
    api/
        mod.rs       -- axum router setup
        handlers.rs  -- Health, Status, List, Logs endpoint handlers
        mcp.rs       -- MCP tool definitions and server (ships in Phase 2!)
    network/
        ssh_proxy.rs -- Unix-to-vsock bridge logic (Phase 4)
        mitm.rs      -- SNI proxy integration
```

### MCP server (ships with daemon from day one)

The MCP server is **not deferred** -- it ships as part of Phase 2. This is critical because:
- Claude Code can use `run_exec` to run commands in the VM during development
- Enables debugging all subsequent phases (SSH, shell, IDE) by executing guest-side commands directly
- `run_exec` reuses the existing `Exec` / `ExecDone` vsock protocol -- minimal new code

**Day-one MCP tools:**

| Tool | Argument Schema | Output |
|------|----------------|--------|
| `provision_sandbox` | `{ name: string, env: { K: V } }` | VM metadata (vm_id, state) |
| `run_exec` | `{ vm_id: string, command: string }` | `{ stdout, stderr, exit_code }` |
| `list_sandboxes` | `{}` | Array of VM metadata |
| `shutdown` | `{ vm_id: string, graceful: boolean }` | Cleanup status |

**Later MCP tools (Phase 4+):**

| Tool | When |
|------|------|
| `read_file` | After SSH/VirtioFS is available |
| `inspect_network` | After network telemetry is queryable via API |

### Multi-VM management

The orchestrator manages a pool of VMs identified by `vm_id`:
- Each VM gets its own session dir: `~/.capsem/sessions/<vm_id>/`
- Session tracking: `session.json` with pid, status, memory_limit_mb, created_at
- PID file: `~/.capsem/sessions/<vm_id>/pid`
- Each VM runs as a dedicated actor/task within the daemon for isolation
- The daemon process itself is per-VM (one fork per `capsem start`)

### HTTP management API (axum)

```
GET  /health           -> { "ok": true }
GET  /status           -> { "vm_id": "dev", "state": "Running", "uptime_secs": 123, "ram_mb": 512 }
GET  /list             -> [{ "vm_id": "dev", ... }, { "vm_id": "test", ... }]
GET  /logs/{vm_id}     -> SSE stream of serial/kernel logs
POST /stop/{vm_id}     -> trigger graceful shutdown
```

Each daemon binds its own Unix socket (`~/.capsem/sessions/<vm_id>/api.sock`) or a TCP port. `capsem status` discovers running daemons by scanning session dirs.

### SIGTERM handler

Must trigger the full guest shutdown handshake:
1. Set `AtomicBool` shutdown flag
2. Orchestrator calls `vm_instance.stop()` which:
   - Sends `HostToGuest::Shutdown { graceful: true }` on control channel
   - Waits for guest sync + unmount + ACPI poweroff (up to 5s)
   - Releases hypervisor resources
   - Cleans up session dir
3. Process exits cleanly

### PID verification

`capsem stop` must verify the PID hasn't been recycled:
- macOS: use `proc_pidinfo(pid, PROC_PIDTBSDINFO)` to check process name
- Only then send SIGTERM, poll for exit (10s timeout, then SIGKILL)

### `BootedVm` struct (shared across shell/daemon/cli)

```rust
struct BootedVm {
    instance: Box<dyn VmInstance>,
    terminal_fd: RawFd,
    control_fd: RawFd,
    ctrl_msg_rx: std::sync::mpsc::Receiver<GuestToHost>,
    mitm_config: Option<Arc<MitmProxyConfig>>,
    rt: tokio::runtime::Runtime,
    session_dir: Option<PathBuf>,
    scratch_path: Option<PathBuf>,
}

impl BootedVm {
    fn shutdown(&mut self) -> Result<()> { /* graceful vsock handshake then stop */ }
}
```

### Files to create
- `crates/capsem-daemon/Cargo.toml` -- new crate, depends on capsem-core + axum + tokio
- `crates/capsem-daemon/src/main.rs` -- daemon entry point, fork/setsid, signal handling
- `crates/capsem-daemon/src/orchestrator.rs` -- multi-VM state machine
- `crates/capsem-daemon/src/api/mod.rs` -- axum router
- `crates/capsem-daemon/src/api/handlers.rs` -- HTTP endpoint implementations
- `crates/capsem-daemon/src/api/mcp.rs` -- MCP tool definitions and server
- `crates/capsem-daemon/src/network/ssh_proxy.rs` -- placeholder for Phase 4
- `crates/capsem-daemon/src/network/mitm.rs` -- SNI proxy integration
- `Cargo.toml` (workspace) -- add `capsem-daemon` member

### Files to modify
- `crates/capsem-app/src/main.rs` -- `capsem start` invokes daemon, `capsem stop`/`capsem status` query it

### Verification
- `capsem start` returns immediately, daemon running in background
- `curl --unix-socket ~/.capsem/sessions/default/api.sock http://localhost/health` returns ok
- MCP `run_exec` executes a command in the VM and returns stdout/exit_code
- `capsem status` shows running VM
- `capsem stop` terminates cleanly (guest filesystem clean)
- Daemon survives terminal close

---

## Phase 3: Shell & Multi-VM (`capsem shell`)

**Goal**: Interactive PTY session targeting a daemon-managed VM. Also connects to existing background VMs.

### `run_shell()` implementation

```rust
fn run_shell(cli_env: &[(String, String)]) -> Result<()>
```

Two modes:
1. **Standalone**: No daemon running -> boot VM directly (like current exec mode but interactive)
2. **Attach**: Daemon running for given vm_id -> connect to its vsock terminal fd via the daemon's Unix socket (future enhancement)

Initial implementation is standalone mode:
1. Call `boot_and_handshake()` to get `BootedVm`
2. Save host terminal via `libc::tcgetattr(STDIN_FILENO)`
3. Set raw mode via `libc::cfmakeraw` + `libc::tcsetattr`
   - Pure passthrough: bracketed paste, mouse reporting, all escape sequences flow through.
     Guest PTY agent's `bridge_loop` passes raw bytes (verified).
4. SIGWINCH handler via self-pipe pattern
5. Send initial `Resize` with terminal dimensions
6. Poll loop: `CFRunLoopRunInMode(0.05)` + `poll([stdin, terminal_fd, sigwinch_pipe], 0)`
7. On EOF: `booted_vm.shutdown()`, restore termios, exit

### CLI dispatch update (main.rs:1184)

```
capsem                                -> Tauri GUI
capsem ui                             -> Tauri GUI
capsem shell [--env K=V]... [--name <id>]  -> run_shell()
capsem start [--env K=V]... [--name <id>]  -> run_daemon()
capsem stop [<id>]                    -> run_stop()
capsem status                         -> run_status()
capsem ssh-config [<id>]              -> print SSH config
capsem [--env K=V]... <command>       -> run_cli() (backward compat)
```

### Files to modify
- `crates/capsem-app/src/main.rs` -- add `run_shell()`, update `main()` dispatch

### Verification
- `capsem shell` gives interactive bash prompt
- `vim`, `top`, `less` work correctly
- Window resize propagates (`stty size`)
- Ctrl-C sends SIGINT, `exit` returns to host shell

---

## Phase 4: SSH Gateway & IDE Integration

**Goal**: VS Code Remote SSH and Google Antigravity connect into VMs. Multi-VM SSH multiplexing via per-vm_id Unix sockets.

### Technical Flow

```
VS Code / Antigravity                              GUEST (Linux VM)
    |
    | ProxyCommand: socat - UNIX:ssh.sock           openssh-server (127.0.0.1:22)
    v                                                      ^
Unix socket (~/.capsem/sessions/<id>/ssh.sock)             | TCP connect
    |                                                      |
    v                                               capsem-ssh-bridge
capsem daemon                                       (on SshBridge msg: connect
  1. accept() unix socket                            vsock:5006 + tcp:22, bridge)
  2. send SshBridge{id} on ctrl ch (vsock:5000) ------>  |
  3. wait for vsock:5006 connection     <-----------  new vsock connection
  4. bridge: unix_fd <-> vsock_fd                     bridge: vsock_fd <-> tcp_fd

  (repeat per SSH channel, 3-5 concurrent for VS Code)
```

Vsock port: **5006** (5003-5005 reserved per roadmap for MCP/AI/file gateways).

### Guest-side changes

**`images/Dockerfile.rootfs`** -- install openssh-server:
```dockerfile
RUN apt-get install -y --no-install-recommends openssh-server && \
    mkdir -p /run/sshd && \
    sed -i 's/#PasswordAuthentication yes/PasswordAuthentication no/' /etc/ssh/sshd_config && \
    echo "ListenAddress 127.0.0.1" >> /etc/ssh/sshd_config && \
    echo "AllowUsers root" >> /etc/ssh/sshd_config && \
    echo "X11Forwarding no" >> /etc/ssh/sshd_config && \
    echo "PermitEmptyPasswords no" >> /etc/ssh/sshd_config && \
    echo "MaxAuthTries 3" >> /etc/ssh/sshd_config
```

**Entropy**: Enable `VZVirtioEntropyDeviceConfiguration` (virtio-rng) in the `AppleVz` backend's machine creation. This seeds `/dev/random` directly from the host, eliminating the need for `haveged` in the guest and keeping the rootfs lean.

**`images/capsem-init`** -- start sshd + bridge:
```sh
if [ -f /capsem-authorized-keys ]; then
    mkdir -p /newroot/root/.ssh
    cp /capsem-authorized-keys /newroot/root/.ssh/authorized_keys
    chmod 700 /newroot/root/.ssh && chmod 600 /newroot/root/.ssh/authorized_keys
fi
chroot /newroot ssh-keygen -A
chroot /newroot /usr/sbin/sshd
capsem-ssh-bridge &
```

**New binary: `capsem-ssh-bridge`** (`crates/capsem-agent/src/ssh_bridge.rs`)
- Listens on control channel for `SshBridge { session_id }` messages
- On each: connect to host vsock:5006 AND TCP 127.0.0.1:22, bridge bidirectionally
- Must handle multiple concurrent bridges (thread-pool or `tokio::spawn` -- VS Code opens 3-5 simultaneously)

### Host-side changes

- `crates/capsem-proto/src/lib.rs` -- add `HostToGuest::SshBridge { session_id: u64 }`
- `crates/capsem-daemon/src/network/ssh_proxy.rs` -- Unix socket listener, per-connection bridge threads
- SSH key management: generate `~/.capsem/ssh/id_ed25519` on first run, inject public key into initrd via `just repack`

### SSH config

`capsem ssh-config dev` outputs:
```
Host capsem-dev
    User root
    IdentityFile ~/.capsem/ssh/id_ed25519
    StrictHostKeyChecking no
    UserKnownHostsFile /dev/null
    ProxyCommand socat - UNIX-CONNECT:$HOME/.capsem/sessions/dev/ssh.sock
```

### VS Code / Antigravity extension

New directory: `vscode-extension/`

| Command | Action |
|---------|--------|
| `Capsem: Start VM` | Run `capsem start` |
| `Capsem: Stop VM` | Run `capsem stop <id>` |
| `Capsem: Connect (Remote SSH)` | Ensure SSH config, trigger Remote SSH |
| `Capsem: Open Terminal` | Integrated terminal with `capsem shell` |
| `Capsem: Fix SSH Config` | Re-generate SSH config, update `~/.ssh/config` |

Sidebar TreeView shows running VMs with status, connect/stop actions.
Extension uses standard VS Code APIs only -- works in both VS Code and Antigravity.
Build as `.vsix` for sideloading. Declare `ms-vscode-remote.remote-ssh` as dependency.

### Diagnostic test updates
- `test_no_sshd` -> `test_sshd_local_only` (127.0.0.1 only, key-only auth)
- New `test_ssh.py` for SSH bridge verification

### Verification
- `ssh capsem-default whoami` returns `root`
- VS Code Remote SSH opens a remote window
- 3-5 concurrent SSH channels work (VS Code typical)
- `capsem-doctor` tests pass
- `just smoke-test` passes

---

## MCP Tool Expansion (ongoing, post-Phase 2)

As phases complete, the MCP server gains additional tools:

| Tool | Available After | Description |
|------|----------------|-------------|
| `provision_sandbox` | Phase 2 | Start a new VM |
| `run_exec` | Phase 2 | Execute command, get stdout/stderr/exit_code |
| `list_sandboxes` | Phase 2 | List running VMs |
| `shutdown` | Phase 2 | Gracefully stop a VM |
| `read_file` | Phase 4 (SSH) | Read file from VM filesystem |
| `inspect_network` | Phase 4 | Query proxied HTTPS requests |

Agents also receive real-time VM telemetry as MCP context:
- VM state, recent network events, resource usage
- Enables autonomous debugging: spin up sandbox, run tests, inspect failures, iterate

---

## Managed GUI (Tauri as daemon client)

The Tauri GUI transitions from managing VMs directly to being a client of the daemon's HTTP/WebSocket API:

**Current**: Tauri setup hook -> boot VM -> own vsock/serial -> emit events to frontend
**Future**: Tauri setup hook -> connect to daemon HTTP API -> subscribe to WebSocket events -> render UI

Benefits:
- Close and reopen GUI without killing active sandboxes
- GUI shows all running VMs (not just the one it started)
- Consistent state between CLI, GUI, and IDE
- Multiple GUIs can connect to the same daemon

This is a migration of `crates/capsem-app/src/main.rs` (Tauri setup hook) and `commands.rs` to use HTTP/WebSocket instead of direct VM access. The frontend Svelte components (`api.ts`, stores) change their data source from Tauri IPC to the daemon API.

---

## Execution Order

```
Phase 1: Hypervisor Abstraction
    |  Define traits, isolate Apple VZ behind feature flag
    |  ~450 LOC refactor, zero behavioral changes
    v
Phase 2: Daemon Core + MCP
    |  capsem-daemon crate: fork/setsid, axum HTTP, orchestrator
    |  MCP server with run_exec, provision_sandbox, list_sandboxes, shutdown
    |  Claude Code can now execute commands in VMs directly!
    v
Phase 3: Shell & Multi-VM
    |  capsem shell (interactive PTY), CLI dispatch
    |  Reuses boot_and_handshake() from Phase 2
    v
Phase 4: SSH & IDE
    |  Guest openssh + ssh-bridge, host Unix socket bridge
    |  VS Code / Antigravity extension
    |  MCP gains: read_file, inspect_network
    |  Requires `just build` for rootfs changes
    v
GUI Migration (ongoing)
    Tauri becomes daemon client
```

Guest-side work for Phase 4 (Dockerfile, init script, bridge binary) can proceed in parallel with Phases 2-3.

## Cross-Cutting Concerns

### Vsock port registry

Create `crates/capsem-proto/src/ports.rs` as single source of truth:
```rust
pub const CONTROL: u32 = 5000;
pub const TERMINAL: u32 = 5001;
pub const SNI_PROXY: u32 = 5002;
// 5003-5005 reserved: MCP gateway, AI gateway, file telemetry
pub const SSH: u32 = 5006;
```

### Binary bloat
openssh-server adds ~2-3MB to rootfs. Initrd is gzip-compressed. Monitor boot time stays <1s.

### capsem-doctor daemon integration
Update `capsem-doctor` to query the daemon's `/health` and `/status` endpoints (when available) rather than checking for local process PIDs. This validates the full platform stack in smoke tests.

### SSH multiplexing
`capsem-ssh-bridge` in the guest must use a thread-pool or `tokio::spawn` to handle multiple concurrent connections (VS Code opens 3-5 simultaneously for extensions, terminal, filesystem).

### macOS persistence (future)
`capsem autostart` to register `~/Library/LaunchAgents/com.capsem.vm.plist` for auto-start on login.

### Antigravity marketplace
Build extension as `.vsix` for sideloading. Verify if Antigravity supports `ms-vscode-remote.remote-ssh` natively.

## Platform Ready Verification Milestone

The project is considered "Platform Ready" when this sequence passes:

```bash
# Phase 2: Daemon boots and responds to health checks
capsem start --name research-env
curl --unix-socket ~/.capsem/sessions/research-env/api.sock http://localhost/health

# Phase 3: Interactive shell works
capsem shell --name research-env

# Phase 4: IDE can connect
code --remote ssh-remote+capsem-research-env /workspace

# Agent can control via MCP
mcp-inspect --server capsem list_sandboxes

# Cleanup
capsem stop research-env
```
