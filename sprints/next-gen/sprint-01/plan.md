# Sprint 1: capsem-service boots one VM with Process Isolation

## Objective
Establish the daemon architecture where `capsem-service` (daemon) manages a single `capsem-process` instance. `capsem-process` acts as the primary security boundary (one per VM), mirroring Chrome's multi-process model. It boots the VM and is designed to further delegate specialized tasks to even lower-privileged sub-processes.

## Security Architecture: The Chrome Metaphor

- **`capsem-service` (The Browser Process):** High privilege. Orchestrates all sandboxes, manages host resources (RAM/CPU), and handles the user-facing API. It never handles untrusted network traffic or guest data directly.
- **`capsem-process` (The Renderer/Sandbox Process):** Medium privilege. One per VM. This is the primary isolation boundary. If one VM's support services crash or are compromised, other VMs and the host remain safe.
- **Future-proofed Delegation:** `capsem-process` is designed to spawn its own child processes for:
    - **MITM Proxy:** Low privilege, network-only access.
    - **MCP Gateway:** Low privilege, policy-gated tool access.
    - **Headless Renderer:** Lowest privilege, handles untrusted web content.

## Tasks

### 1. Refactor `capsem-core`
- [ ] Create `crates/capsem-core/src/vm/boot.rs` and move `boot_vm` logic from `capsem-app` there.
- [ ] Create `crates/capsem-core/src/vm/terminal.rs` and move `TerminalOutputQueue` there.
- [ ] Create `crates/capsem-core/src/vm/registry.rs` and move `VmInstance` (renamed to `SandboxInstance`) and `VmNetworkState` there.
- [ ] Ensure all necessary types are exported from `capsem-core`.

### 2. Create `capsem-process` crate (The Sandbox)
- [ ] `crates/capsem-process/Cargo.toml`
- [ ] `crates/capsem-process/src/main.rs`:
  - Takes VM configuration (RAM, CPU, image paths) via CLI args.
  - Boots the VM on the main thread (required for Apple VZ).
  - Sets up vsock, serial, MITM proxy, and MCP gateway (initially in-process, but with a clear path to sub-process delegation).
  - Listens on a per-VM Unix Domain Socket (UDS) for control/terminal from `capsem-service`.
  - Drops privileges where possible (e.g., using macOS sandbox profiles).

### 3. Create `capsem-service` crate (The Orchestrator)
- [ ] `crates/capsem-service/Cargo.toml`
- [ ] `crates/capsem-service/src/main.rs`:
  - Daemonize (fork/setsid).
  - Listen on `~/.capsem/service.sock` for CLI commands.
  - Implement `provision_sandbox`:
    - Fork and exec `capsem-process` with a unique ID and its own sandbox profile.
    - Track the PID and the UDS for the instance.
  - Implement `list_sandboxes`, `get_status`, `shutdown`.
  - `ResourceManager` (Layer 1): track RAM/CPU allocations across all `capsem-process` children.
  - Recovery loop: rediscover running `capsem-process` instances on startup via `~/.capsem/run/instances/`.

### 4. Create `capsem` (CLI) binary
- [ ] `crates/capsem/Cargo.toml`
- [ ] `crates/capsem/src/main.rs`:
  - `capsem start [--name <id>] [--ram <GB>] [--cpu <count>]`
  - `capsem stop [<id>]`
  - `capsem list`
  - `capsem status [<id>]`

### 5. Verification
- [ ] `capsem-service` starts and daemonizes.
- [ ] `capsem start` boots a VM via `capsem-process`.
- [ ] `capsem list` shows the running VM with its PID and resource usage.
- [ ] `capsem stop` shuts it down cleanly by signaling the child process.
- [ ] Kill `capsem-service` and restart: it should re-adopt the running `capsem-process` via the filesystem state.

## Key Files & Context
- `crates/capsem-core`: shared logic for VM boot and management.
- `~/.capsem/run/instances/`: directory containing `<vm_id>.json` (state) and `<vm_id>.sock` (control).
- macOS Sandbox: `capsem-process` should eventually be wrapped in a restrictive profile.
