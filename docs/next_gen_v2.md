# Revised next-gen.md

## Context

Capsem evolves from a fire-and-forget CLI / Tauri GUI into a multi-VM daemon platform with checkpoint/branching, cross-platform support, and IDE integration. VFS (virtio-fs) for checkpoint storage is assumed done and out of scope.

**Key design decisions:**
- VFS is checkpoint plumbing only -- no host-side file access API. `read_file` goes through guest SSH (Phase 6), never by reading VFS directories.
- Ephemeral mode stays default; persistent/checkpointable is opt-in (`--persistent`)
- Every checkpoint captures full VM state (filesystem + CPU + memory)
- macOS = Apple VZ, Linux/ChromeOS = crosvm
- Single user, multi VM. One daemon manages all VMs for one user.
- **Auth: SSH keys are the universal identity.** No tokens, no separate CAs. User's SSH key authenticates to HTTPS API, WSS terminal, and SSH for VS Code. Daemon verifies via custom rustls `ClientCertVerifier` matching SPKI against `authorized_keys`.

---

## Phase Structure

| Phase | What |
|---|---|
| 1 | Hypervisor Abstraction (traits + Apple VZ backend) |
| 2 | crosvm Linux Backend |
| 3 | Daemon + MCP + Menu Bar |
| 4 | UI -- Browser Chrome |
| 5 | Shell |
| 6 | MITM SSH + IDE |
| 7 | Chat UI (to be designed) |

---

## Security Architecture (cross-cutting, all phases)

### Identity: SSH Keys

The user's SSH key is the single credential for all daemon interactions. No tokens. No passwords. No separate CA infrastructure.

**Client side (Tauri app / CLI / remote client):**

1. Read user's SSH private key (e.g., `~/.ssh/id_ed25519`)
2. `ssh-key` crate parses the OpenSSH private key
3. `rcgen` crate generates an ephemeral X.509 certificate in memory, signed with the SSH key
4. Present this cert during mTLS handshake to daemon

```rust
use rcgen::{CertificateParams, KeyPair};
use ssh_key::PrivateKey;

let ssh_key_str = std::fs::read_to_string("~/.ssh/id_ed25519")?;
let ssh_key = PrivateKey::from_openssh(&ssh_key_str)?;
let key_pair = KeyPair::from_ed25519_bytes(
    ssh_key.key_data().ed25519().unwrap().as_ref()
)?;
let params = CertificateParams::new(vec!["capsem-client".to_string()]);
let cert = rcgen::Certificate::from_params(params)?;
let cert_der = cert.serialize_der_with_signer(&key_pair)?;
// Pass cert_der + key_pair to tungstenite/reqwest as mTLS identity
```

**Daemon side:**

1. On startup, load authorized public keys from `~/.capsem/authorized_keys`
   - Default: reads local `~/.ssh/*.pub` if no explicit authorized_keys exists
   - Can add remote keys via `capsem authorize <pubkey>`
2. Custom `SshAuthorizedKeysVerifier` implementing `rustls::server::danger::ClientCertVerifier`
3. Extracts SPKI from client's X.509 cert via `x509-parser`, ignores CA/expiry/signature
4. Byte-for-byte match against authorized SSH public keys

```rust
struct SshAuthorizedKeysVerifier {
    authorized_public_keys: Vec<Vec<u8>>, // raw pubkey bytes from authorized_keys
}

impl ClientCertVerifier for SshAuthorizedKeysVerifier {
    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, Error> {
        let (_, parsed) = x509_parser::parse_x509_certificate(end_entity.as_ref())
            .map_err(|_| Error::General("Invalid X.509 DER".into()))?;
        let client_pub_key = parsed.tbs_certificate.subject_pki.subject_public_key.data;
        if self.authorized_public_keys.contains(&client_pub_key.to_vec()) {
            Ok(ClientCertVerified::assertion())
        } else {
            Err(Error::General("Key not in authorized_keys".into()))
        }
    }
    // supported_verify_schemes: use ring default provider
    // root_hint_subjects: empty (no CAs)
}
```

**Crates:** `ssh-key`, `rcgen` (client side), `x509-parser`, `rustls` (daemon side)

### Connection transports

| Transport | Auth mechanism | Use case |
|---|---|---|
| Unix socket | Filesystem perms (owner-only) | Local CLI, local Tauri UI |
| HTTPS / WSS | mTLS with ephemeral X.509 cert (SSH key inside) | Remote API, remote terminal |
| SSH | Standard SSH key auth via `russh` | VS Code Remote, IDE integration |

All three verify the same identity: the user's SSH public key. One key, three transports.

### MITM model

Capsem MITMs both HTTPS and SSH from the guest:

| Protocol | How |
|---|---|
| **HTTPS** (existing) | Guest → iptables redirect → capsem-net-proxy → vsock:5002 → host MITM proxy. Terminate TLS (per-domain cert from capsem CA), inspect HTTP, apply policy, forward to real upstream. |
| **SSH** (Phase 6) | VS Code → daemon SSH server (`russh`) → authenticate with user's SSH key → MITM: inspect commands/file transfers, enforce policy, log telemetry → separate SSH session to guest openssh-server via vsock:5006. **One-way only: outside → VM. The VM cannot initiate SSH/SFTP outward.** |

Both follow the same pattern: terminate the connection, inspect/policy/log, bridge to the real destination.

**SSH direction enforcement (critical security invariant):** The SSH bridge is strictly inbound-only (host → guest). The guest VM must never be able to initiate SSH/SFTP connections to the outside world. **This cannot rely on iptables** -- the AI running inside the VM can modify iptables rules. Enforcement must be architectural:
- The `capsem-ssh-bridge` guest binary only bridges vsock:5006 → localhost:22 (inbound direction)
- The guest has no vsock client capability for port 5006 (only the host can connect to the guest's vsock listener)
- No SSH client keys are provisioned inside the guest
- The daemon SSH server (`russh`) only accepts connections from the host side, never from the guest

### VM isolation

- Each VM: separate vsock CID, separate VFS dirs, separate session DB
- Network policy per-VM (inherited from config, carried forward on branch)
- Checkpoint dirs: same filesystem permissions as session dir
- Single-user model: all VMs owned by one user, daemon enforces no cross-user access

### Agent identification

The MITM proxy already identifies which AI agent is running from HTTP traffic to provider APIs (Anthropic, Google, OpenAI). This data is already in session.db (`model_calls` table). No additional detection needed -- just surface what's already captured.

- **UI**: Tab labels show agent icon/name. VM list shows agent per session.
- **Security**: Corp policy can restrict allowed agents (`[agents] allowed = ["claude", "gemini"]`).
- **Health endpoint**: Per-VM `agent` field derived from model_calls data.

### Subprocess sandboxing

**Every service runs as its own sandboxed process.** Today everything runs as async tasks in one process -- this is a Phase 3 blocker. The daemon must be a process supervisor, not a monolith. Each service gets its own process, its own sandbox profile, its own resource limits, and its own crash domain.

**Mandatory process separation (Phase 3):**

| Process | Responsibility | IPC |
|---------|---------------|-----|
| `capsem-daemon` | Orchestrator, HTTP/WS API, auth, menu bar, subprocess supervisor | Parent process |
| `capsem-mitm` | HTTPS MITM proxy (TLS termination, HTTP inspection, AI traffic parsing) | Unix socket / pipe |
| `capsem-mcp-gateway` | MCP tool routing (builtin tools, external servers, policy) | Unix socket / pipe |
| `capsem-monitor` | Telemetry DB writer, session index, FS monitoring, auto-snapshots | Unix socket / pipe |

**Future processes (later phases):**

| Process | Phase | Responsibility |
|---------|-------|---------------|
| `capsem-ssh-bridge` | Phase 6 | MITM SSH proxy (russh server, SSH telemetry) |
| `capsem-web-renderer` | Phase 7 | Headless browser for chat UI / web rendering |
| `capsem-mcp-server` | Phase 3 | External MCP server exposing capsem tools to AI agents |

**Process abstraction (`SubprocessSpec` trait):**

Every managed process implements the same interface:

1. **Identity**: binary path, args, environment
2. **Sandbox profile**: macOS `sandbox-exec` profile / Linux seccomp + namespaces. Principle of least privilege -- MITM proxy needs network, MCP gateway does not, monitor only needs filesystem.
3. **Resource limits**: CPU, memory, file descriptor caps. Configurable in settings per process type.
4. **IPC**: How the daemon communicates with the subprocess (Unix socket, pipe, shared fd). Defined per process type.
5. **Restart policy**: max restarts, backoff (exponential with cap), crash count tracked in health endpoint.
6. **Lifecycle**: Start, health check, graceful stop (SIGTERM + timeout), force kill (SIGKILL). SIGTERM cascades from daemon to all children.
7. **Logging**: Each subprocess logs to its own file or structured channel. Daemon aggregates.

**`SubprocessManager`** in daemon owns a registry of `SubprocessSpec` instances. It handles:
- Startup ordering (monitor before mitm, mitm before mcp-gateway)
- Health monitoring (periodic liveness checks)
- Crash recovery with exponential backoff
- Graceful shutdown in reverse startup order
- Resource accounting (aggregate CPU/memory across all children)

**Why separate processes, not threads:**
- **Crash isolation**: MITM proxy crash doesn't kill the MCP gateway or telemetry writer.
- **Sandbox granularity**: Each process gets its own OS-level sandbox. MITM proxy needs network access, MCP gateway does not.
- **Resource limits**: OS-enforced per-process limits. A runaway parser can't starve the telemetry writer.
- **Upgradability**: Individual processes can be restarted without bouncing the whole daemon.
- **Auditability**: Each process has its own PID, its own log stream, its own resource usage visible in `ps`/Activity Monitor.

### Terminal notifications

Terminal applications emit notifications via OSC escape sequences (bell `\x07`, iTerm2 `\e]9;`, OSC 777, etc.). These must propagate through the stack:

1. **Guest → host**: Terminal data (vsock:5001) already passes raw bytes including escape sequences
2. **Host → daemon**: CoalesceBuffer preserves escape sequences in terminal data
3. **Daemon → UI**: WebSocket terminal stream carries the raw data to xterm.js
4. **xterm.js → UI notification**: xterm.js `onBell` callback → Svelte notification component → OS-level notification (Tauri notification API or browser Notification API)
5. **Menu bar**: Bell/notification events also surface as a badge on the menu bar tray icon

*Phase 3*: Daemon forwards terminal data including escape sequences (already works). *Phase 4*: UI hooks `onBell` and routes to notification system.

### User confirmation for dangerous actions

Certain operations require explicit user confirmation before proceeding:

1. **MCP tool authorization**: When an AI agent requests a new MCP tool that hasn't been approved, the UI shows a confirmation dialog. Agent waits for user response.
2. **Dangerous VM operations**: Delete VM, restore checkpoint (overwrites current state), stop running VM from remote client.
3. **Policy violations**: If a tool call or network request hits a "confirm" policy rule (not "allow" or "deny" but "ask"), the UI prompts the user.

**Architecture**: Daemon emits a `ConfirmationRequest` event (via SSE/WebSocket) with request details. UI shows a modal/notification. User approves or denies. Daemon receives the response and proceeds or rejects the operation. Timeout: configurable (default 60s), defaults to deny on timeout.

**Menu bar fallback**: If no UI is connected, confirmation requests surface as macOS notifications with action buttons. If no response mechanism is available (headless daemon), configurable default (deny or allow, per-operation-type, set in settings).

*Phase 3*: Daemon implements `ConfirmationRequest` event + response channel. *Phase 4*: UI implements confirmation modal + notification routing. *Deferred*: Per-operation-type confirmation policies in corp.toml.

### Corp manifest and feature restrictions

Corp config (`/etc/capsem/corp.toml`) properly enforced:

1. **Merge semantics** (existing): Corp fields override user fields entirely. Unspecified fields fall through to user, then defaults.
2. **Feature gates**: Any feature can be disabled via corp settings. Architecture: each feature checks `settings.is_enabled("feature.name")` which consults the merged config.
3. **Restricted features** (configurable in user.toml or corp.toml):
   - `ssh.enabled` -- enable/disable SSH server (Phase 6)
   - `checkpoints.enabled` -- enable/disable checkpoint/branch
   - `remote.enabled` -- enable/disable remote daemon access
   - `mcp.enabled` -- enable/disable MCP server
   - `agents.allowed` -- list of allowed AI agents
   - `notifications.enabled` -- enable/disable terminal notifications
   - Corp can lock any of these and users cannot override

4. **Corp manifest validation**: On daemon startup, validate corp.toml schema (fail fast on malformed config). Log corp policy hash in audit log.

*Phase 3*: Settings registry extended with feature gates. Corp merge enforced at daemon startup. *Phase 4*: UI reflects locked/restricted features (greyed out, "Restricted by organization" tooltip).

### Telemetry and OpenTelemetry readiness

The daemon's telemetry architecture is designed for OTEL from the start, even if OTEL export is not enabled by default.

**Instrumentation points** (all phases):
- VM lifecycle events (boot, pause, checkpoint, stop)
- MITM proxy requests (domain, method, path, status, latency, policy decision)
- MCP tool calls (tool name, vm_id, agent, duration, result)
- SSH sessions (commands, file transfers, bytes, duration) -- Phase 6
- API calls (endpoint, client fingerprint, duration, result)
- Confirmation requests (operation, response, latency)

**Agent tracking**: Every telemetry event includes `agent_id` (which AI agent triggered it). Enables per-agent cost tracking, security auditing, and usage analytics.

**Internal format**: All telemetry goes through a unified `TelemetryEvent` enum, written to session DB (existing) AND optionally exported via OTEL.

**OTEL export** (opt-in, feature-gated):
- Crates: `opentelemetry`, `opentelemetry-otlp`, `tracing-opentelemetry`
- Bridges existing `tracing` spans -- OTEL is an additional subscriber, not a replacement
- Configurable endpoint, protocol (gRPC/HTTP), auth headers
- Corp can require OTEL (`telemetry.required = true` in corp.toml)

*Phase 3*: `TelemetryEvent` enum, session DB writes, audit.log, OTEL subscriber (disabled by default). *Deferred*: Dashboard templates, alerting rules, compliance reporting.

---

## Phase 1: Hypervisor Abstraction

Define `Hypervisor` and `VmInstance` traits. Isolate all Apple VZ code behind an `AppleVz` backend. Zero functional changes -- pure refactor.

### Trait design

```rust
pub trait Hypervisor: Send + Sync {
    fn boot(&self, config: VmConfig) -> Result<Box<dyn VmInstance>>;
}

pub trait VmInstance: Send + Sync {
    fn stop(&self) -> Result<()>;
    fn pause(&self) -> Result<()>;
    fn resume(&self) -> Result<()>;
    fn save_state(&self, path: &Path) -> Result<()>;
    fn restore_state(&self, path: &Path) -> Result<()>;
    fn cid(&self) -> u32;
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

pub enum VmState { Created, Booting, Running, Paused, Stopped, Error }
```

### File structure

```
crates/capsem-core/src/
  hypervisor/
    mod.rs              -- traits + VmState
    apple_vz/
      mod.rs            -- AppleVzHypervisor
      machine.rs        -- from vm/machine.rs
      boot.rs           -- from vm/boot.rs
      serial.rs         -- from vm/serial.rs
      vsock.rs          -- VZ listener delegation
  vm/
    config.rs           -- stays (platform-agnostic)
    vsock.rs            -- CoalesceBuffer + VsockConnection (platform-agnostic)
```

### Feature gate

```toml
[features]
default = ["apple-vz"]
apple-vz = ["objc2-virtualization", "objc2", "objc2-foundation", "block2", "dispatch2", "core-foundation-sys"]
```

### Key constraint

Remove `inner_vz()` from `machine.rs:143-144`. No `objc2` or `Virtualization` symbols may appear outside `src/hypervisor/apple_vz/`.

### Files to modify
- `vm/machine.rs` → `hypervisor/apple_vz/machine.rs`
- `vm/boot.rs` → `hypervisor/apple_vz/boot.rs`
- `vm/serial.rs` → `hypervisor/apple_vz/serial.rs`
- `vm/vsock.rs` → split: VZ parts to `apple_vz/vsock.rs`, generic parts stay
- `capsem-core/src/lib.rs` → export `hypervisor` module
- `capsem-app/src/state.rs` → `Box<dyn VmInstance>` instead of `VirtualMachine`
- `capsem-app/src/main.rs` → use trait methods, remove `inner_vz()` calls

### Verification
- `cargo test --workspace` passes
- `just run "capsem-doctor"` boots and passes (zero behavioral changes)
- `cfg(not(feature = "apple-vz"))` compiles (no VZ symbols in trait definitions)

---

## Phase 2: crosvm Linux Backend

Implement the `Hypervisor` trait for Linux/ChromeOS using crosvm.

### Why crosvm
- **Rust-native**: no C FFI, no libvirt
- **Has everything**: vsock, virtio-fs, virtio-blk, virtio-console, snapshot/restore
- **Security-focused**: built for Chrome OS sandboxing
- **Chromebook native**: crosvm IS the system VMM on Chrome OS

### Backend mapping

| Trait method | Apple VZ (macOS) | crosvm (Linux) |
|---|---|---|
| `boot()` | `VZVirtualMachine::start` | `crosvm run` |
| `stop()` | `VZVirtualMachine::stop` | `crosvm stop` |
| `pause()` | `VZVirtualMachine::pause` | `crosvm suspend` |
| `resume()` | `VZVirtualMachine::resume` | `crosvm resume` |
| `save_state()` | `saveMachineStateToURL` | `crosvm snapshot` |
| `restore_state()` | `restoreMachineStateFromURL` | `crosvm restore` |
| vsock | `VZVirtioSocketDevice` | `--vsock` flag |
| virtio-fs | `VZVirtioFileSystemDevice` | `--shared-dir` flag |

### File structure

```
crates/capsem-core/src/hypervisor/
    mod.rs              # traits (Phase 1)
    apple_vz/           # macOS (Phase 1)
    crosvm/
        mod.rs          # CrosvmHypervisor
        process.rs      # crosvm process management
        snapshot.rs     # snapshot/restore
```

### Feature gate

```toml
[features]
default = ["apple-vz"]
apple-vz = [...]
crosvm = []
```

Same guest image (kernel, squashfs, capsem-init). Only host VMM changes. Add `images/defconfig.x86_64` if supporting x86.

### Verification
- `cargo test --workspace --features crosvm` passes on Linux
- `capsem start` boots VM via crosvm
- Checkpoint + branch work identically to macOS
- Same capsem-doctor tests pass inside crosvm guest

---

## Phase 3: Daemon + MCP + Menu Bar

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
    +-- Hypervisor (AppleVz or crosvm): owns VM, vsock, serial
    +-- MITM proxy: HTTPS inspection
    +-- Auth: SshAuthorizedKeysVerifier (loads ~/.capsem/authorized_keys)
    +-- HTTPS/WSS API (axum + rustls): mTLS with SSH key verification
    +-- MCP server: checkpoint, branch, provision, exec
    +-- macOS menu bar: NSStatusItem
    +-- CFRunLoop pumping (VZ requirement on macOS)
```

### SSH key lifecycle

**Initial wizard** (Phase 4 UI, but daemon supports it from day one):
1. Scan `~/.ssh/` for existing key pairs (`id_ed25519`, `id_rsa`, `id_ecdsa`)
2. Present found keys to user, let them select which to authorize
3. If no keys found, offer to generate `~/.capsem/ssh/id_ed25519`
4. Write selected public keys to `~/.capsem/authorized_keys`

**Periodic rescan on launch**:
1. On every daemon startup, rescan `~/.ssh/*.pub` for new keys
2. Compare against `~/.capsem/authorized_keys`
3. If new keys found, log them but do NOT auto-add (security: user must explicitly authorize)
4. Surface new-key notification via menu bar / UI notification

**Key storage**:
- `~/.capsem/authorized_keys` -- authorized public keys (SSH format, one per line)
- `~/.capsem/ssh/id_ed25519` -- capsem-generated key pair (if user has no existing keys)
- Per-session guest keys: generated ephemerally, injected via initrd, never stored on host

### Daemon auth on startup

1. Load `~/.capsem/authorized_keys` (auto-populate from `~/.ssh/*.pub` on first run only)
2. Rescan `~/.ssh/*.pub` for new keys (notify, don't auto-add)
3. Generate self-signed TLS server cert (ephemeral, in-memory)
4. Configure rustls `ServerConfig` with `SshAuthorizedKeysVerifier`
5. Bind Unix socket (local) + TLS listener (remote)

### MCP tools

**VM lifecycle:**

| Tool | Description |
|---|---|
| `provision_sandbox` | Start a new VM (`persistent: bool`, `name: string`) |
| `list_sandboxes` | List all VMs with state, uptime, provider info |
| `shutdown` | Graceful stop (`vm_id`, `graceful: bool`) |
| `pause` | Pause VM (freeze CPU, keep memory) |
| `resume` | Resume paused VM |

**Execution & files:**

| Tool | Description |
|---|---|
| `run_exec` | Execute command in VM, return `{ stdout, stderr, exit_code }` |
| `read_file` | Read file contents from VM (via `run_exec "cat <path>"` initially, SSH/sftp in Phase 6) |
| `write_file` | Write file to VM (via `run_exec "cat > <path>"` initially, SSH/sftp in Phase 6) |
| `list_files` | List directory contents (`path`, `recursive: bool`) |

**Checkpoint & branching:**

| Tool | Description |
|---|---|
| `checkpoint` | Create full VM state checkpoint (`name: string`) |
| `branch` | Fork from checkpoint into new VM (`checkpoint_id`, `name`) |
| `list_checkpoints` | List checkpoint tree for a VM |
| `restore` | Rollback VM to a previous checkpoint |

**Observability:**

| Tool | Description |
|---|---|
| `get_logs` | Get serial/boot logs for a VM (`vm_id`, `lines: int`, `follow: bool`) |
| `screenshot` | Capture terminal screenshot as text (ANSI dump of current terminal buffer) |
| `inspect_network` | Query proxied HTTPS telemetry (`vm_id`, `domain`, `last_n`) |
| `get_status` | Detailed VM status: state, uptime, RAM, disk, network stats |

**UI tools (Tauri-specific):**

| Tool | Description |
|---|---|
| `open_ui` | Launch or focus the Capsem GUI window |
| `close_ui` | Close the GUI window |
| `screenshot_ui` | Capture a screenshot of the Capsem app window (PNG, returned as base64) |
| `get_ui_state` | Current UI state: active tab, open panel section, content mode, window size |
| `navigate_ui` | Switch tab, open/close panel, change panel section, toggle content mode |
| `resize_ui` | Resize the app window |

These tools require the Tauri app to be running. The daemon sends UI commands to the Tauri app via a reverse WebSocket channel (the Tauri app already connects to the daemon -- the daemon pushes UI command requests on that same connection, Tauri app executes them and returns results). If the Tauri app is not connected, these tools return an error.

**Taskbar / menu bar tools:**

| Tool | Description |
|---|---|
| `get_tray_status` | List of VMs shown in the menu bar with their states |
| `tray_action` | Trigger a tray menu action: new VM, pause, resume, stop, open UI |
| `set_tray_badge` | Set a notification badge or status text on the tray icon |

These run directly in the daemon (the tray is part of the daemon process, not Tauri).

**Telemetry tools:**

| Tool | Description |
|---|---|
| `query_telemetry` | Query session telemetry: model calls, tool calls, net events, fs events. Supports filters (vm_id, time range, event type) |
| `get_session_summary` | Aggregated stats for a session: total tokens, cost, tool count, domains accessed, duration |
| `list_sessions` | List all sessions with basic metadata (id, vm name, start/end time, state) |
| `export_telemetry` | Export session telemetry as JSON (for external analysis) |

Telemetry tools query the session SQLite databases (`~/.capsem/sessions/<vm_id>/web.db` and future `main.db`). These run in the daemon with no UI dependency.

**Design notes:**
- `screenshot` captures the terminal buffer as text (not a pixel screenshot -- VMs have no GUI). Returns the current visible terminal content, useful for AI agents to understand what's on screen.
- `read_file`/`write_file` use `run_exec` as transport (e.g., `cat`, `base64`). These are MCP tools -- separate from SSH/SFTP (Phase 6). Both capabilities coexist.
- `get_logs` with `follow: true` streams via SSE, useful for AI agents monitoring boot progress or debugging.
- All tools require `vm_id` (except `provision_sandbox` and `list_sandboxes`).

**Security consideration -- `read_file`/`write_file`:** These MCP tools give external AI agents the ability to read and write arbitrary files inside the VM. While the VM is sandboxed (writes are ephemeral, network is air-gapped), this still warrants careful policy:
- Default: enabled (the VM is a sandbox, file access is the point)
- Corp policy can restrict to specific paths or disable entirely via `[[mcp.rules]]`
- All file operations are logged to session telemetry (path, size, direction)
- These tools are distinct from SSH/SFTP (Phase 6) -- MCP file tools go through `run_exec`, SSH file access goes through the MITM SSH bridge. Both are valid access paths with independent policy controls.

### HTTP API

All endpoints except `/health` use `/{vm_id}/action` URL pattern (VM ID first for easy log filtering/grep).

```
GET  /health                     -> infra-compatible health check (see below)
GET  /vms                        -> list all VMs

GET  /{vm_id}/status             -> { vm_id, state, uptime, ram }
GET  /{vm_id}/logs               -> SSE stream of serial logs
POST /{vm_id}/stop               -> graceful shutdown
POST /{vm_id}/pause              -> pause VM
POST /{vm_id}/resume             -> resume VM
DELETE /{vm_id}                  -> delete VM + checkpoints
POST /{vm_id}/checkpoint         -> create checkpoint
POST /{vm_id}/branch/{cp_id}     -> fork from checkpoint
GET  /{vm_id}/checkpoints        -> checkpoint tree
POST /{vm_id}/restore/{cp_id}    -> restore to checkpoint
WS   /{vm_id}/terminal           -> bidirectional terminal I/O
```

**Health endpoint** (`GET /health`): Compatible with Kubernetes probes, load balancers, and monitoring infra.
```json
{
  "status": "ok",
  "version": "0.9.0",
  "uptime_secs": 3600,
  "auth": "ssh-key",
  "host": {
    "memory_total_mb": 32768,
    "memory_available_mb": 16384,
    "disk_total_gb": 500,
    "disk_available_gb": 120,
    "cpu_cores": 10
  },
  "vms": [
    {
      "id": "dev",
      "state": "running",
      "uptime_secs": 1800,
      "ram_mb": 4096,
      "cpu_cores": 4,
      "disk_used_mb": 512,
      "checkpoint_count": 3,
      "agent": "claude",
      "storage_mode": "persistent"
    },
    {
      "id": "research",
      "state": "paused",
      "uptime_secs": 7200,
      "ram_mb": 4096,
      "cpu_cores": 4,
      "disk_used_mb": 1024,
      "checkpoint_count": 1,
      "agent": "gemini",
      "storage_mode": "persistent"
    }
  ]
}
```
Returns `200 OK` when healthy, `503 Service Unavailable` when degraded. No auth required on `/health` (infra tools can't do mTLS). Includes host resources, per-VM details, and agent identification.

**Audit logging**: Every API call logged to `~/.capsem/audit.log` with timestamp, client identity (SSH pubkey fingerprint), endpoint, VM ID, result, duration. Structured JSON, one line per call. Required for security product -- full accountability for all daemon operations.

Local: Unix socket at `~/.capsem/daemon.sock` (no TLS needed, filesystem perms).
Remote: TLS with mTLS client auth (SSH key verification).

### macOS menu bar tray

```
Capsem
─────────────
● dev        Running   [Pause] [Stop]
◉ research   Paused    [Resume] [Stop]
─────────────
+ New VM...
Open Capsem UI
─────────────
Quit
```

- Runs as part of daemon process (not Tauri window)
- Works without GUI open
- Remote: shows "Connected to <host>"

### OpenTelemetry

The daemon exports structured telemetry via OpenTelemetry (OTLP) for enterprise monitoring. This replaces ad-hoc logging with a standard protocol that integrates with Datadog, Splunk, Grafana, and any OTEL-compatible backend.

**Traces:**
- Per-request traces through the MITM proxy (domain, method, path, status, latency)
- MCP tool call traces (tool name, vm_id, duration, result)
- VM lifecycle traces (boot, pause, checkpoint, stop)
- SSH session traces (commands executed, files transferred)

**Metrics:**
- `capsem.vm.count` -- gauge: active VMs by state (running, paused, stopped)
- `capsem.vm.boot_duration` -- histogram: boot time
- `capsem.proxy.requests` -- counter: HTTPS requests by domain, method, policy decision (allowed/denied)
- `capsem.mcp.tool_calls` -- counter: MCP tool invocations by tool name
- `capsem.tokens.total` -- counter: total tokens across all VMs
- `capsem.policy.violations` -- counter: policy denials by rule

**Logs:**
- Structured event stream (same data as `audit.log` but via OTLP)
- Each log includes: timestamp, VM ID, client identity (SSH key fingerprint), action, result

**Configuration** (`~/.capsem/user.toml` or corp.toml):
```toml
[telemetry]
enabled = true
endpoint = "https://otel-collector.corp.com:4317"  # OTLP gRPC endpoint
protocol = "grpc"                                    # or "http"
headers = { "Authorization" = "Bearer <token>" }     # optional auth
service_name = "capsem-daemon"
```

**Crates:** `opentelemetry`, `opentelemetry-otlp`, `tracing-opentelemetry` (bridges existing `tracing` spans to OTEL). The daemon already uses `tracing` -- OTEL is an additional subscriber layer, not a replacement.

**Default:** disabled. Enterprise deploys enable it via corp.toml. When disabled, no OTEL dependencies are loaded (feature-gated).

### Enterprise Policy Endpoint

Enterprise deployments need centralized policy management beyond static MDM-distributed TOML files. The daemon supports pulling policy from a remote endpoint.

**Pull model** (daemon polls, no inbound connectivity required):
1. Daemon checks `[enterprise.policy_endpoint]` in corp.toml on startup
2. Periodically polls the endpoint (configurable interval, default 5 minutes)
3. Receives a JSON policy document (equivalent to corp.toml in JSON)
4. Merges with local corp.toml (remote overrides local)
5. Policy changes take effect immediately for new connections (in-flight connections finish under old policy)

**Configuration** (corp.toml only -- users cannot set this):
```toml
[enterprise]
policy_endpoint = "https://capsem-policy.corp.com/v1/policy"
poll_interval_secs = 300
auth_header = "Bearer <corp-token>"           # or mTLS
device_id = "auto"                             # derived from machine identity
```

**Policy document format:**
```json
{
  "version": 2,
  "updated_at": "2026-03-17T10:00:00Z",
  "network": {
    "allowed_domains": ["api.anthropic.com", "*.googleapis.com"],
    "blocked_domains": ["*"],
    "rules": [...]
  },
  "mcp": {
    "disabled_tools": ["write_file"],
    "path_restrictions": ["/workspace/*"]
  },
  "telemetry": {
    "endpoint": "https://otel-collector.corp.com:4317",
    "required": true
  }
}
```

**Security:**
- Policy endpoint is authenticated (bearer token or mTLS)
- Response is validated (schema check, version monotonicity)
- If endpoint is unreachable, daemon continues with last-known-good policy (cached locally)
- Corp.toml `policy_endpoint` cannot be overridden by user.toml
- Audit log records every policy fetch (endpoint, response hash, changes applied)

### Auto-start on login (mandatory)

The daemon auto-starts on login so VMs are always available:
- **macOS**: `~/Library/LaunchAgents/com.capsem.daemon.plist` -- `capsem autostart enable/disable`
- **Linux**: `~/.config/systemd/user/capsem.service` -- `capsem autostart enable/disable`
- Daemon survives logout (setsid already handles this)
- `capsem autostart status` shows whether auto-start is configured

### SIGTERM handler

1. Set `AtomicBool` shutdown flag
2. Send `HostToGuest::Shutdown { graceful: true }` on control channel
3. Wait for guest sync + unmount (5s timeout)
4. Release hypervisor resources, clean up
5. Exit cleanly

### Files to create
- `crates/capsem-daemon/Cargo.toml`
- `crates/capsem-daemon/src/main.rs` -- fork/setsid, signals
- `crates/capsem-daemon/src/orchestrator.rs` -- multi-VM state machine
- `crates/capsem-daemon/src/auth.rs` -- `SshAuthorizedKeysVerifier`, key loading
- `crates/capsem-daemon/src/api/mod.rs` -- axum router + rustls config
- `crates/capsem-daemon/src/api/handlers.rs` -- HTTP endpoints
- `crates/capsem-daemon/src/api/mcp.rs` -- MCP tools
- `crates/capsem-daemon/src/api/terminal_ws.rs` -- WebSocket terminal bridge
- `Cargo.toml` workspace -- add member

### Files to modify
- `capsem-app/src/main.rs` -- `capsem start/stop/status` dispatch

### Verification
- `capsem start` returns immediately, daemon running + menu bar visible
- Local: `curl --unix-socket ~/.capsem/daemon.sock http://localhost/health`
- Remote: mTLS connection with SSH-key-derived cert succeeds
- Unauthenticated remote connection rejected
- MCP `checkpoint` + `branch` work end-to-end
- `capsem stop` terminates cleanly
- Daemon survives terminal close

---

## Phase 4: UI -- Browser Chrome

The entire UI adopts a **browser-chrome metaphor**. No left sidebar. VM sessions are tabs. Controls live in a toolbar. Stats/settings open in a toggleable side panel.

### 4.1 Layout hierarchy

```
BrowserShell (flex-col, h-screen)
+-- TabBar                              VM tabs at the very top (like Chrome tabs)
+-- Toolbar                             Navigation bar (like Chrome address bar)
|   +-- VmControls                      Left: [Pause] [Checkpoint] [Restore] [Stop]
|   +-- AddressBar                      Center: state dot + session name
|   +-- ContentModeToggle               Center-right: [Terminal | Chat]
|   +-- ToolbarActions                  Right: [Panel] [Settings] [Theme]
+-- ContentArea (flex-row, flex-1)
|   +-- SidePanel (toggleable, right)   Stats / Checkpoints / Settings
|   +-- MainContent (flex-1)
|       +-- TerminalView                Always mounted per tab, visibility toggled
|       +-- ChatView                    Chat UI (like Claude Desktop)
|       +-- NewTabPage                  VM list shown on "+" tab
+-- StatsBar                            Bottom bar: tokens | tools | cost (terminal mode only)
```

### 4.2 Tab bar

Each open VM = a browser tab. Plus a "+" new-tab button.

```
+-----------------------------------------------------------------------+
| [+]  [* dev]  [* research]  [experiment]                              |
+-----------------------------------------------------------------------+
```

- **State dot per tab**: blue = running, purple = stopped, orange = booting, grey = not created
- **Close (x)**: appears on hover. If VM running, confirm dialog ("Stop VM and close?")
- **Click**: switches active tab. Terminal stays mounted (visibility toggle) to preserve xterm.js state
- **"+"**: opens NewTabPage (VM list + create new)
- **Keyboard**: Cmd+T = new tab, Cmd+W = close, Cmd+1-9 = switch to tab N

Each tab owns its own `<capsem-terminal>` web component instance (~2-5MB each, fine for 1-5 tabs).

### 4.3 Toolbar

```
[||] [Save] [Restore v] [Stop]  |  * Running  "dev"  |  [Terminal] [Chat]  |  [Panel] [Gear] [Theme]
```

**Left cluster -- VM controls** (maps to browser back/forward/reload/stop):
- **Pause/Resume** toggle (like back button position)
- **Checkpoint** = save snapshot (like forward button)
- **Restore** dropdown = recent checkpoints to roll back to (like reload button)
- **Stop** = stop VM (like stop/X button)

**Center -- Address bar**: state dot + session name in a rounded `bg-base-200` container. Session name is editable (click to rename).

**Center-right -- Content mode toggle**: segmented `[Terminal] [Chat]` button. Hidden on NewTabPage.

**Right cluster -- Actions**:
- **Panel toggle**: opens/closes the side panel (stats icon)
- **Settings gear**: switches panel to settings section
- **Theme toggle**: existing light/dark toggle

### 4.4 Content modes: Terminal vs Chat

**Terminal** (default): xterm.js fills content area. StatsBar visible at bottom. Terminal always mounted per tab (visibility toggled, never destroyed).

**Chat** (future phase, to be designed): a Claude Desktop-style chat interface where the AI provider's tools are sandboxed in the VM. Content mode toggle in toolbar reserves the "Chat" position but shows "Coming soon" placeholder. Full design deferred to a dedicated phase.

Switching modes: toolbar toggle sets `contentMode` on active tab. Both Terminal and Chat coexist per tab -- switching hides one and shows the other.

### 4.5 Side panel (toggleable, right side)

Toggled by the panel icon in the toolbar. Slides in from the right (~380px), like Chrome's side panel. Terminal stays left-anchored so text doesn't reflow when panel opens. When closed, content gets full width.

**Three section tabs at the top of the panel:**

| Section | Content |
|---|---|
| **Stats** | AI / Tools / Network / Files sub-tabs. Reuses existing tab components adapted for panel width. |
| **Checkpoints** | Vertical timeline tree. Each node: timestamp, name, model, tokens. [Restore] and [Branch] buttons per checkpoint. [+ Checkpoint Now] at top. |
| **Settings** | Reuses existing settings tree + SettingsSection/McpSection. SubMenu collapses into section headers. |

Panel state persists per tab (open/closed, active section).

### 4.6 New Tab page (VM list)

When "+" is clicked or no tabs are open, content area shows the VM list:

```
+--------------------------------------------------+
|  Capsem                                          |
|                                                   |
|  [+ Create New Sandbox]                          |
|                                                   |
|  Recent Sessions                                  |
|  +----------------------------------------------+ |
|  | [*] claude-refactor    running    1h ago      | |
|  | [*] gemini-research    stopped    5h ago      | |
|  | [*] codex-debug        booting    just now    | |
|  +----------------------------------------------+ |
|                                                   |
|  {if no API key:}                                |
|  "Configure an AI provider to get started"       |
|  [Configure Providers]                           |
+--------------------------------------------------+
```

- Click running session -> opens as tab
- Click stopped session -> offers restore from checkpoint or start fresh
- "Create New" -> creates VM, opens as tab
- First-run wizard absorbed here (no separate WizardView)

### 4.7 Frontend architecture changes

**Data source migration**: Frontend switches from direct Tauri IPC to daemon API.

| Current (Tauri IPC) | New (Daemon API) |
|---|---|
| `serial_input()` Tauri command | WebSocket `ws://daemon/terminal/{vm_id}` |
| `vm-state-changed` Tauri event | SSE `GET /events/{vm_id}` |
| `vm_status()` Tauri command | `GET /{vm_id}/status` |
| `get_settings_tree()` Tauri command | `GET /settings` |

**Auth**: same as before (local mTLS with SSH key, transparent to user).

**New stores:**

| Store | Purpose |
|---|---|
| `tabs.svelte.ts` | Tab list, active tab, content mode per tab. Replaces `sidebarStore`. |
| `panel.svelte.ts` | Side panel open/closed, active section (stats/checkpoints/settings), stats sub-tab. |
| `checkpoints.svelte.ts` | Checkpoint tree for active VM. |

**Deleted stores:**
- `sidebar.svelte.ts` -- replaced by `tabs` + `panel`

**Modified stores:**
- `vmStore` -- multi-VM aware: `vmStates: Map<string, VmState>` instead of single state
- `statsStore` -- polling becomes per-VM (active tab's session), `activeTab` moves to `panelStore`

**New components:**

| Component | Purpose |
|---|---|
| `BrowserShell.svelte` | Root layout (replaces App.svelte's inner layout) |
| `TabBar.svelte` | Horizontal tab bar + new-tab button |
| `Tab.svelte` | Single tab: state dot, name, close button |
| `Toolbar.svelte` | Full toolbar row |
| `VmControls.svelte` | Left toolbar: pause, checkpoint, restore, stop |
| `AddressBar.svelte` | Center toolbar: state + session name |
| `ContentModeToggle.svelte` | Terminal/Chat segmented switch |
| `ToolbarActions.svelte` | Right toolbar: panel toggle, settings, theme |
| `SidePanel.svelte` | Sliding panel container with section tabs |
| `NewTabPage.svelte` | VM list + create new (replaces VmListView) |
| `ChatView.svelte` | Chat UI (stub in 4a, full in 4b) |
| `panel/StatsPanel.svelte` | Stats adapted for panel width |
| `panel/CheckpointsPanel.svelte` | Checkpoint timeline |
| `panel/SettingsPanel.svelte` | Settings adapted for panel |

**Deleted components:**
- `Sidebar.svelte` -- absorbed by TabBar + Toolbar
- `StatusBar.svelte` -- absorbed by Toolbar + StatsBar
- `WizardView.svelte` -- absorbed by NewTabPage

**Modified components:**
- `App.svelte` -- gutted, delegates to BrowserShell
- `TerminalView.svelte` -- per-tab instances (one `<capsem-terminal>` per tab)
- `StatsBar.svelte` -- "Stats >" now calls `panelStore.show('stats')`

**Mock data additions:**
- `VmSummary[]` -- 3 mock VMs (running, stopped, booting) with provider info
- `Checkpoint[]` -- 3 mock checkpoints for first VM
- New mock API methods: `listVms()`, `createVm()`, `stopVm()`, `pauseVm()`, `listCheckpoints()`, `createCheckpoint()`, `restoreCheckpoint()`

### 4.8 Implementation phases

**Phase 4a** (core browser chrome, mock multi-VM):
1. Create `tabsStore`, `panelStore` stores
2. Build BrowserShell, TabBar, Toolbar, SidePanel components
3. Rewire App.svelte to use BrowserShell
4. Move Stats into SidePanel
5. Move Settings into SidePanel
6. Build NewTabPage with mock VM list
7. Delete Sidebar.svelte, StatusBar.svelte
8. Add mock data for VMs and checkpoints
9. Verify in `just ui`

**Phase 4b** (real multi-VM + chat):
1. Build ChatView (full implementation)
2. Wire tabsStore to daemon HTTP API
3. Build CheckpointsPanel with real checkpoint API
4. Per-tab terminal instances
5. Multi-VM vmStore refactor

### 4.9 Verification
- Opening Capsem shows NewTabPage (VM list)
- "+" opens new tab page, "Create" creates VM
- Click VM -> opens as tab with terminal
- Toolbar controls (pause/checkpoint/restore/stop) work
- Content mode toggle switches between Terminal and Chat
- Panel toggle shows/hides side panel with Stats/Checkpoints/Settings
- Tab switching preserves terminal state (xterm.js not reflowed)
- `just ui` shows mock VM list, checkpoints, and all chrome elements
- Remote: connect to daemon on different machine, all features work

---

## Phase 5: Shell

Interactive PTY session: `capsem shell [--name <id>]`

Two modes:
1. **Standalone**: No daemon → boot VM directly
2. **Attach**: Daemon running → connect to VM's terminal WebSocket

Implementation: `boot_and_handshake()` → save termios → raw mode via `cfmakeraw` → SIGWINCH handler via self-pipe → poll loop → shutdown → restore termios.

### CLI dispatch

```
capsem                          -> GUI (Tauri, daemon client)
capsem ui                       -> GUI (alias)
capsem shell [--name <id>]      -> interactive PTY
capsem start [--name <id>]      -> start background VM (daemon)
capsem stop [<id>]              -> stop VM
capsem status                   -> list running VMs
capsem authorize <pubkey>       -> add SSH pubkey to authorized_keys
capsem ssh-config [<id>]        -> print SSH config snippet
capsem autostart enable|disable|status -> manage login auto-start
```

### Verification
- `capsem shell` gives interactive bash prompt
- `vim`, `top`, `less` work
- Window resize propagates
- Ctrl-C sends SIGINT, `exit` returns to host

---

## Phase 6: MITM SSH + IDE

### MITM SSH architecture

Same pattern as MITM HTTPS: terminate, inspect, policy, bridge.

```
VS Code / User                  Capsem Daemon                    Guest VM
    |                               |                               |
    | ssh capsem-dev                |                               |
    |----> daemon SSH server ------>|                               |
    |      (russh, port 2222)       |                               |
    |      auth: user's SSH key     |                               |
    |                               | Terminate SSH session         |
    |                               | Inspect commands/sftp/        |
    |                               |   file transfers              |
    |                               | Apply policy                  |
    |                               | Log telemetry to session DB   |
    |                               |                               |
    |                               | New SSH session to guest      |
    |                               |----> vsock:5006 ------------->|
    |                               |      (capsem-ssh-bridge)      |
    |                               |      auth: per-session key    | openssh-server
    |                               |                               |
    |<---- bridge <-----------------|<----- bridge <----------------|
```

### Guest-side

- `openssh-server` installed in rootfs (`Dockerfile.rootfs`)
- Config: `ListenAddress 127.0.0.1`, `PasswordAuthentication no`, `AllowUsers root`
- Per-session SSH key generated by capsem, injected via initrd into `/root/.ssh/authorized_keys`
- `capsem-ssh-bridge`: listens for `SshBridge` control message on vsock:5000, bridges vsock:5006 -> TCP 127.0.0.1:22 (inbound only)
- Handles 3-5 concurrent SSH channels (VS Code typical)
- **No SSH client keys provisioned in guest** -- the guest cannot initiate outbound SSH/SFTP
- **No outbound vsock capability on port 5006** -- the bridge only accepts inbound connections from the host

### Host-side (daemon)

- `russh` crate: SSH server accepting user's SSH key
- **Username = VM name**: `ssh dev@localhost:2222` routes to VM "dev" (see VM routing section above)
- For each SSH channel: inspect + policy + open separate SSH client session to guest via vsock
- Telemetry: log commands, file transfers, sftp operations to session DB (`ssh_events` table)
- Policy: configurable command blocklist, file path restrictions

### SSH session recording (mandatory, day one)

Every SSH session is fully recorded. Non-negotiable for a security product.

- **Terminal I/O**: Raw byte stream recorded with timestamps (enables session replay)
- **Commands**: Shell commands captured and logged individually
- **File transfers**: SFTP/SCP operations logged (path, direction, size, hash)
- **Storage**: `ssh_events` table in session DB + raw recording files in session dir
- **UI**: Session replay viewer in the Stats panel (Phase 4). Scrubber timeline, play/pause, search.
- **Audit**: All SSH activity included in audit.log and OTEL telemetry

### VM routing via SSH username

The SSH username IS the VM name. The daemon's `russh` server routes connections based on the authenticated username:
- `ssh dev@localhost -p 2222` → connects to VM "dev"
- `ssh research@localhost -p 2222` → connects to VM "research"
- Unknown username → connection rejected with error listing available VMs

This means VS Code can connect to different VMs by changing only the SSH user.

### SSH port

The daemon's SSH server runs on its own port (default 2222, configurable via `ssh.port` in user.toml). This avoids conflicting with the system SSH server on port 22. Port is advertised in the health endpoint and `capsem ssh-config` output.

### SSH config

`capsem ssh-config dev` outputs:
```
Host capsem-dev
    User dev
    Port 2222
    HostName localhost
    IdentityFile ~/.ssh/id_ed25519
    StrictHostKeyChecking no
    UserKnownHostsFile /dev/null
```

For remote daemon: `HostName` points to daemon host. For all VMs: `capsem ssh-config` (no arg) outputs config for every running VM.

### MCP tool updates

| Tool | Change |
|---|---|
| `inspect_network` | Extended to include SSH telemetry alongside HTTPS |

**Note:** MCP `read_file`/`write_file` keep their `run_exec`-based transport. They are not replaced by SFTP -- these are separate access paths. SFTP exists only if VS Code Remote SSH requires it (VS Code's remote filesystem extension uses SFTP). If SFTP is not needed for IDE integration, it is not added.

### VS Code extension

`vscode-extension/` directory:

| Command | Action |
|---|---|
| `Capsem: Start VM` | `capsem start` |
| `Capsem: Stop VM` | `capsem stop <id>` |
| `Capsem: Connect` | Ensure SSH config, trigger Remote SSH |
| `Capsem: Open Terminal` | `capsem shell` in integrated terminal |

### Verification
- `ssh capsem-dev whoami` returns `root`
- VS Code Remote SSH opens a remote window
- 3-5 concurrent SSH channels work
- MITM logs SSH commands/file transfers in session DB
- Policy blocks disallowed commands
- capsem-doctor passes (update `test_sshd_local_only`)

---

## Execution Order

```
Phase 1: Hypervisor Abstraction
    |  Traits (pause/resume/save/restore), Apple VZ backend
    v
Phase 2: crosvm Linux Backend
    |  Same traits, crosvm implementation
    |  Both backends validated before building daemon
    v
Phase 3: Daemon + MCP + Menu Bar
    |  Orchestrator, mTLS auth (SSH keys), HTTP/WS API
    |  MCP: provision, exec, checkpoint, branch, restore
    |  macOS menu bar tray
    v
Phase 4: UI -- Browser Chrome
    |  Tab bar (VM sessions as tabs), toolbar (controls + address bar)
    |  Terminal/Chat content toggle, toggleable side panel (stats/checkpoints/settings)
    |  NewTabPage (VM list), mTLS auth from Tauri
    |  Remote daemon support
    v
Phase 5: Shell
    |  capsem shell (interactive PTY)
    v
Phase 6: MITM SSH + IDE
    |  russh SSH server on daemon, MITM to guest openssh (inbound only)
    |  VS Code Remote SSH, SSH telemetry + policy enforcement
    v
Phase 7: Chat UI (to be designed)
    |  Claude Desktop-style chat where AI tools are sandboxed in VM
    |  Provider selector, streaming, MCP tool integration
```

Phases 5 and 6 can proceed in parallel. Phase 7 design deferred.

---

## Vsock Port Registry

```
5000  Control messages (resize, heartbeat, exec, shutdown, ssh-bridge)
5001  Terminal PTY I/O
5002  SNI proxy (HTTPS MITM)
5003  MCP gateway (stdio-to-vsock relay)
5006  SSH bridge (inbound only, MITM SSH, Phase 6)
```

---

## Mandatory Features by Phase

Items that must ship with their phase, not deferred.

| Feature | Phase | Notes |
|---|---|---|
| **OS-level notifications** | Phase 4 | Terminal bell + notification events → macOS/Linux desktop notifications via Tauri notification API. Required, not optional. |
| **Auto-start on login** | Phase 3 | macOS: `~/Library/LaunchAgents/com.capsem.daemon.plist`. Linux: systemd user service `~/.config/systemd/user/capsem.service`. Daemon survives logout. |
| **SSH session recording** | Phase 6 | Every SSH session fully recorded from day one: commands, file transfers, terminal I/O. Session replay in UI. Stored in session DB (`ssh_events` table). Non-negotiable for a security product. |
| **VFS checkpoint diff** | Phase 4 | Diff tool for comparing checkpoint snapshots. Being implemented soon. |

---

## Deferred Features (architecture in place, implementation later)

These features are NOT in scope for Phases 1-6 but the architecture explicitly supports them.

| Feature | Foundation | What's needed to ship |
|---|---|---|
| **Per-subprocess seccomp/sandbox profiles** | Phase 3 `SubprocessManager` | Write seccomp profiles, test per-OS |
| **Web renderer subprocess** | Phase 3 `SubprocessManager` | New subprocess type, browser automation |
| **OTEL alerting rules** | Phase 3 OTEL subscriber | Alert configs for Datadog/PagerDuty/etc. |
| **Per-operation confirmation policies in corp.toml** | Phase 3 `ConfirmationRequest` | Schema extension, policy engine rules |
| **Compliance reporting** | Phase 3 audit.log + OTEL | Report templates, export formats |
| **Advanced MCP path restrictions** | Phase 3 MCP policy engine | Per-tool path patterns in policy config |
| **Chat UI (Phase 7)** | Phase 4 content mode toggle + `ChatView` stub | Full chat implementation, provider integration |
| **Remote daemon auto-discovery** | Phase 3 health endpoint + SSH | mDNS/Bonjour advertisement, UI scanner |
| **Keyless VM** (no AI keys in image) | Phase 3 MCP gateway + MITM proxy | Proxy injects auth headers, keys never enter guest |
| **Agent switching mid-session** | Phase 3 agent tracking | Hot-swap agent identity, update telemetry |

**Architectural invariant**: No deferred feature should require changing the Phase 1-3 foundations. If a deferred feature would need a new vsock port, protocol message, or trait method, it must be reserved now even if unused.
