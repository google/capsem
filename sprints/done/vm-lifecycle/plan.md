# VM Lifecycle Sprint -- Plan

## What

Add guest-initiated lifecycle control to Capsem VMs. Users inside a VM can run `shutdown`, `halt`, `poweroff`, or `suspend` and have those commands flow through the host service's existing code paths. Also injects VM identity (name, ID) so `hostname` and `uname -n` return meaningful values.

This is a meta sprint covering: VM identity injection, a multi-call guest system binary, end-to-end shutdown, the quiescence protocol (fsfreeze before snapshot ops), Apple VZ pause/save/restore, agent reconnect after resume, and suspend/resume wired through service + CLI + MCP.

## Why

1. **No shutdown**: VMs cannot be stopped from inside. `exit` leaves PID 1 in a spin loop. There is no `/sbin/shutdown`.
2. **No identity**: `hostname` returns a generic Debian default. Users and scripts inside the VM have no way to know which sandbox they're in.
3. **No suspend**: The spike proved Apple VZ save/restore works (730ms, 54MB for 2GB VM) but it's not wired into the system. No quiescence protocol exists. No agent reconnect after restore.
4. **Shutdown must use the service path**: If guest-initiated shutdown bypasses the service, ephemeral cleanup, persistent registry updates, and future quiescence all break. One code path for all stops.

## Key Decisions

1. **Guest shutdown = `capsem stop`**: Same behavior -- ephemeral VMs destroyed, persistent VMs preserved. The guest sends `ShutdownRequest` up to the service, which calls the same `shutdown_vm_process()` it uses for `capsem stop`.
2. **5-second shutdown timeout**: Replaces the current 150ms in `shutdown_vm_process()`. Gives time for `sync + fsfreeze` once quiescence exists. Agent gets 2s for bash cleanup before the host force-kills.
3. **Guest suspend = `capsem suspend`**: Guest sends `SuspendRequest` up to the service. Service orchestrates quiescence + Apple VZ save_state. Same code path as the API endpoint.
4. **Multi-call binary (busybox pattern)**: Single `capsem-sysutil` binary dispatches on `argv[0]`. Symlinks created by `capsem-init` at boot. One cross-compile target, one initrd entry.
5. **capsem-sysutil opens its own vsock**: Does NOT go through the agent. This means shutdown works even if the agent is hung.
6. **Identity via existing SetEnv**: No protocol changes for identity -- `CAPSEM_VM_ID` and `CAPSEM_VM_NAME` injected as env vars by the service. Agent calls `sethostname()` after boot config.
7. **Quiescence protocol**: `PrepareSnapshot` / `SnapshotReady` / `Unfreeze` messages. Agent runs `sync + fsfreeze -f /` on prepare, `fsfreeze -u /` on unfreeze. Required before suspend, branch, and rewind.
8. **Agent reconnect after restore**: After VZ restore, vsock fds are broken. Agent detects EOF, re-connects with exponential backoff. Existing retry-on-connect code is the template.
9. **Apple VZ only for now**: KVM suspend/resume is ~11-15 days of separate work (next-gen S9, deferred). This sprint adds `pause()`/`resume()`/`save_state()`/`restore_state()` to the `VmHandle` trait with default `unimplemented!()` for KVM.

## Sub-Sprint Details

### T0: Protocol + Identity

New protocol messages and VM identity injection. Foundation for everything else.

**Protocol additions:**

```rust
// GuestToHost (guest -> host via vsock:5000)
ShutdownRequest,        // guest wants to stop
SuspendRequest,         // guest wants to suspend

// HostToGuest (host -> guest via vsock:5000)  
// Shutdown already exists
PrepareSnapshot,        // quiescence: sync + fsfreeze
Unfreeze,               // resume filesystem I/O

// GuestToHost
SnapshotReady,          // quiescence ack: safe to proceed
```

**IPC additions:**

```rust
// ProcessToService (capsem-process -> capsem-service)
ShutdownRequested { id: String },
SuspendRequested { id: String },

// ServiceToProcess (capsem-service -> capsem-process)
Suspend { checkpoint_path: String },
Resume,                 // distinct from current cold-boot resume
PrepareSnapshot,        // forwarded to guest
Unfreeze,               // forwarded to guest
```

**Identity injection:**

Service adds `--env CAPSEM_VM_ID={id}` and `--env CAPSEM_VM_NAME={name_or_id}` when spawning capsem-process. Agent calls `sethostname()` from `CAPSEM_VM_NAME` after boot config. No protocol changes.

**Files:**
- `crates/capsem-proto/src/lib.rs` -- add HostToGuest + GuestToHost variants
- `crates/capsem-proto/src/ipc.rs` -- add ServiceToProcess + ProcessToService variants
- `crates/capsem-service/src/main.rs` -- inject CAPSEM_VM_ID/CAPSEM_VM_NAME as --env
- `crates/capsem-agent/src/main.rs` -- sethostname after boot env applied

### T1: capsem-sysutil Binary

Multi-call binary for guest system commands. Dispatches on `argv[0]`.

**Commands:**

| argv[0] | Action |
|---------|--------|
| `shutdown`, `halt`, `poweroff` | Send `GuestToHost::ShutdownRequest` via vsock:5000, print "Shutting down sandbox...", exit 0 |
| `suspend` | Send `GuestToHost::SuspendRequest` via vsock:5000, print "Suspending sandbox...", exit 0 |
| `reboot` | Print "reboot is not supported in capsem sandbox", exit 1 |

**Implementation:** ~200 lines of Rust. Opens vsock CID 2 port 5000 directly (not through the agent). Sends one MessagePack frame. Reads optional ack. Exits.

Accepts common flags but ignores most: `shutdown -h now` (default), `shutdown -r` (reboot, error), `shutdown --help`. Just enough to not confuse scripts.

**Files:**
- `crates/capsem-agent/src/bin/capsem_sysutil.rs` -- new binary
- `crates/capsem-agent/Cargo.toml` -- add `[[bin]]` entry
- `guest/artifacts/capsem-init` -- create symlinks after binary deploy
- `justfile` -- add to `_pack-initrd` cross-compile + copy

### T2: Shutdown Flow (End-to-End)

Wire guest-initiated shutdown through the service's existing stop path.

**Flow:**
```
capsem-sysutil (guest) --vsock:5000--> capsem-agent control loop
  agent sees ShutdownRequest, forwards to capsem-process via existing ctrl channel
capsem-process --IPC--> ProcessToService::ShutdownRequested { id }
capsem-service receives ShutdownRequested
  calls shutdown_vm_process() -- SAME code path as handle_stop()
  shutdown_vm_process() sends ServiceToProcess::Shutdown back to process
capsem-process receives Shutdown, sends HostToGuest::Shutdown to agent
capsem-agent: sync, SIGTERM bash, 2s wait, break
capsem-init: while loop keeps PID 1 alive
host: after 5s timeout, SIGKILL process
service: cleanup (ephemeral destroy / persistent preserve)
```

**Note:** capsem-sysutil sends `ShutdownRequest` directly on vsock:5000, but the agent's control loop is already reading from that fd. Two options:
- **(A)** capsem-sysutil sends to agent, agent forwards to process. Simple, uses existing channel.
- **(B)** capsem-sysutil opens a second vsock connection on port 5000. Process sees it as a new control connection.

**Decision: Option A.** The agent's control channel is the single authorized communication path. capsem-sysutil sends a `GuestToHost::ShutdownRequest` frame on vsock:5000 -- but since the agent owns that fd, the sysutil binary needs a local IPC mechanism to ask the agent to send it.

**Revised approach:** capsem-sysutil writes to a well-known Unix socket inside the guest (`/run/capsem-agent.sock`) that the agent listens on. Or simpler: capsem-sysutil writes a trigger file (`/run/capsem-shutdown`) and the agent watches for it. Or simplest: capsem-sysutil opens its **own** vsock connection to CID 2 port 5000. The host capsem-process already accepts multiple vsock connections on port 5000 -- it just needs to handle `ShutdownRequest` from any of them.

**Final decision:** capsem-sysutil opens its own vsock:5000 connection. The host capsem-process control message router already loops on received messages. We add a second accepted connection on port 5000 in the process, OR we add a dedicated port (5004) for lifecycle commands. Using a dedicated port is cleaner -- no multiplexing concerns.

**Port 5004: lifecycle commands.** capsem-process listens on vsock:5004 for lifecycle requests from guest system binaries. Single message per connection (connect, send ShutdownRequest/SuspendRequest, close).

**Shutdown timeout:** Increase `shutdown_vm_process()` from 150ms to 5s. Sequence:
1. Service sends `ServiceToProcess::Shutdown`
2. Process sends `HostToGuest::Shutdown` to agent
3. Agent: `sync`, SIGTERM bash, 2s cleanup
4. Service: 5s total timeout, then SIGKILL
5. Cleanup: remove from instances, ephemeral destroy / persistent preserve

**Agent handling of HostToGuest::Shutdown (currently unimplemented at main.rs:742):**
```rust
Ok(HostToGuest::Shutdown) => {
    eprintln!("[capsem-agent] received Shutdown from host");
    unsafe { libc::sync(); }
    let _ = nix::sys::signal::kill(child_pid, nix::sys::signal::Signal::SIGTERM);
    std::thread::sleep(std::time::Duration::from_secs(2));
    break; // exit control loop, capsem-init's while loop keeps PID 1 alive
}
```

**Files:**
- `crates/capsem-process/src/main.rs` -- accept vsock:5004, handle ShutdownRequest by sending ProcessToService::ShutdownRequested. Handle HostToGuest::Shutdown by forwarding to agent.
- `crates/capsem-service/src/main.rs` -- handle ProcessToService::ShutdownRequested by calling shutdown_vm_process(). Increase timeout to 5s.
- `crates/capsem-agent/src/main.rs` -- implement HostToGuest::Shutdown handler (sync + kill bash + break)

### T3: VmHandle Trait + Apple VZ Pause/Save

Extend the hypervisor trait with checkpoint operations. Implement for Apple VZ using the proven spike code.

**Trait additions:**
```rust
pub trait VmHandle: Send {
    fn stop(&self) -> Result<()>;
    fn state(&self) -> VmState;
    fn serial(&self) -> &dyn SerialConsole;
    fn as_any(&self) -> &dyn std::any::Any;

    // New -- checkpoint lifecycle
    fn pause(&self) -> Result<()> { Err(anyhow!("pause not supported on this platform")) }
    fn resume(&self) -> Result<()> { Err(anyhow!("resume not supported on this platform")) }
    fn save_state(&self, path: &std::path::Path) -> Result<()> { Err(anyhow!("save_state not supported")) }
    fn restore_state(&self, path: &std::path::Path) -> Result<()> { Err(anyhow!("restore_state not supported")) }
    fn supports_checkpoint(&self) -> bool { false }
}
```

**Apple VZ implementation:** Port the spike code (branch `spike/checkpoint-restore`) into `AppleVzMachine`:
- `pause()`: `VZVirtualMachine.pause(completionHandler:)` -- must call on main thread
- `resume()`: `VZVirtualMachine.resume(completionHandler:)` -- must call on main thread
- `save_state(path)`: must be paused first. `VZVirtualMachine.saveMachineStateTo(_:completionHandler:)` (macOS 14+)
- `restore_state(path)`: must be stopped. `VZVirtualMachine.restoreMachineStateFrom(_:completionHandler:)` (macOS 14+)
- `supports_checkpoint()`: `VZVirtualMachine.canSaveState` (macOS 14+), false on earlier

**KVM:** Default trait impls return errors. KVM suspend/resume is deferred (next-gen S9).

**Minimum macOS version guard:** `save_state`/`restore_state` require macOS 14+. Runtime check via `supports_checkpoint()`. Compile-time: use `#[cfg]` or availability check.

**Files:**
- `crates/capsem-core/src/hypervisor/mod.rs` -- extend VmHandle trait
- `crates/capsem-core/src/hypervisor/apple_vz/machine.rs` -- implement pause/resume/save_state/restore_state
- `crates/capsem-core/src/hypervisor/apple_vz/mod.rs` -- wire AppleVzHandle to machine methods

### T4: Quiescence Protocol

Guest quiescence brings the VM to a clean state before any snapshot operation. Required before suspend, branch, and rewind.

**Protocol (added in T0):**
```
Host -> Guest: PrepareSnapshot
Guest agent:   sync && fsfreeze -f /
Guest -> Host: SnapshotReady
[host does operation]
Host -> Guest: Unfreeze
Guest agent:   fsfreeze -u /
```

**Agent implementation:**
```rust
Ok(HostToGuest::PrepareSnapshot) => {
    eprintln!("[capsem-agent] preparing for snapshot...");
    // Flush all dirty pages
    unsafe { libc::sync(); }
    // Freeze all mounted filesystems (overlayfs -> ext4 -> squashfs)
    let exit = std::process::Command::new("fsfreeze")
        .args(["-f", "/"])
        .status();
    match exit {
        Ok(s) if s.success() => {
            send_guest_msg(control_fd, &GuestToHost::SnapshotReady)?;
        }
        _ => {
            send_guest_msg(control_fd, &GuestToHost::Error {
                id: 0,
                message: "fsfreeze failed".into(),
            })?;
        }
    }
}
Ok(HostToGuest::Unfreeze) => {
    let _ = std::process::Command::new("fsfreeze")
        .args(["-u", "/"])
        .status();
    eprintln!("[capsem-agent] filesystem unfrozen");
}
```

**Timeout:** If the agent doesn't send `SnapshotReady` within 10s, the host aborts the operation and sends `Unfreeze` as a safety measure.

**Files:**
- `crates/capsem-agent/src/main.rs` -- handle PrepareSnapshot (sync + fsfreeze) and Unfreeze
- `crates/capsem-process/src/main.rs` -- quiescence orchestration helper: send PrepareSnapshot, wait for SnapshotReady with timeout, do operation, send Unfreeze

### T5: Agent Reconnect

After VZ restore, vsock fds are broken (connections torn down by stop/restore). The agent must detect this and re-establish connections.

**Current behavior:** Agent connects to vsock at boot, never reconnects. If the fd breaks (EOF or error), the control loop exits and the agent dies.

**New behavior:** Agent wraps the main loop in a reconnect loop:
```rust
loop {
    match run_agent_session() {
        Ok(AgentExit::Shutdown) => break,      // clean shutdown, don't reconnect
        Ok(AgentExit::Suspended) => {
            // vsock will break during save. Wait for restore.
            eprintln!("[capsem-agent] suspended, waiting for reconnect...");
            // Exponential backoff: 100ms, 200ms, 400ms, ... up to 5s
            reconnect_with_backoff()?;
        }
        Err(e) => {
            eprintln!("[capsem-agent] session error: {e}, attempting reconnect...");
            reconnect_with_backoff()?;
        }
    }
}
```

**Reconnect flow:**
1. Agent detects EOF on vsock fd (control or terminal)
2. Closes all vsock fds
3. Waits with exponential backoff (100ms initial, 5s max)
4. Reconnects to vsock CID 2 ports 5000+5001
5. Sends `GuestToHost::Ready` again
6. Host sends new boot config (or abbreviated reconnect handshake)
7. Agent sends `BootReady`
8. If filesystem was frozen: `fsfreeze -u /`
9. Resume terminal bridge

**Process side:** capsem-process must also handle the reconnection. After VZ restore + resume, it waits for the agent to re-connect on vsock. The existing vsock accept loop handles this naturally -- new connections arrive on the channel.

**Files:**
- `crates/capsem-agent/src/main.rs` -- wrap main loop in reconnect logic, add backoff
- `crates/capsem-process/src/main.rs` -- handle re-accepted vsock connections after restore

### T6: Suspend/Resume Service Flow

Wire suspend/resume end-to-end through the service, using the same code-path principle as shutdown.

**Suspend flow (service-initiated or guest-initiated):**
```
Service: handle_suspend(id) OR receives SuspendRequested { id }
  1. Send ServiceToProcess::PrepareSnapshot to process
  2. Process sends HostToGuest::PrepareSnapshot to agent
  3. Agent: sync + fsfreeze, sends SnapshotReady
  4. Process forwards SnapshotReady to service
  5. Service sends ServiceToProcess::Suspend { checkpoint_path }
  6. Process: vm.pause(), vm.save_state(path), vm.stop()
  7. Process sends ProcessToService::StateChanged { state: "Suspended" }
  8. Service updates instance state, frees RAM allocation from ResourceManager
```

**Resume flow (service-initiated):**
```
Service: handle_resume(id) -- enhanced to detect suspended vs stopped
  If suspended (checkpoint file exists):
    1. Re-spawn capsem-process with --checkpoint-path flag
    2. Process: boot VM, vm.restore_state(path), vm.resume()
    3. Agent reconnects (T5), sends Ready
    4. Process sends Unfreeze to agent
    5. Agent: fsfreeze -u /
    6. Process sends StateChanged { state: "Running" }
  If stopped (no checkpoint, persistent):
    Same as current cold-boot resume
```

**Checkpoint storage:**
```
~/.capsem/run/instances/{id}/checkpoint.vzsave   # Apple VZ checkpoint file
```

**Service API:**
- `POST /suspend/{id}` -- new endpoint
- `POST /resume/{name}` -- enhanced to detect and use checkpoint if present

**State tracking:** `InstanceInfo` gets a new field:
```rust
checkpoint_path: Option<PathBuf>,  // Some if suspended, None if running/stopped
```

And `PersistentVmEntry` needs:
```rust
suspended: bool,
checkpoint_path: Option<PathBuf>,
```

**Files:**
- `crates/capsem-service/src/main.rs` -- add handle_suspend, enhance handle_resume, update InstanceInfo/PersistentVmEntry
- `crates/capsem-service/src/api.rs` -- add SuspendRequest type if needed
- `crates/capsem-process/src/main.rs` -- handle Suspend (quiesce + pause + save + stop), handle --checkpoint-path for warm resume
- `crates/capsem-proto/src/ipc.rs` -- already added in T0

### T7: Guest-Initiated Suspend

Wire `capsem-sysutil suspend` through the service.

**Flow:**
```
Guest: user types "suspend"
  -> capsem-sysutil opens vsock:5004, sends GuestToHost::SuspendRequest
  -> capsem-process receives on lifecycle port
  -> sends ProcessToService::SuspendRequested { id } to service
  -> service calls handle_suspend() -- same code path as POST /suspend/{id}
  -> quiescence + save_state + stop
  -> VM is suspended, user sees nothing (VM is gone)
  -> capsem resume <name> brings it back (warm restore)
```

**capsem-sysutil output:**
```
$ suspend
[capsem] Suspending sandbox... (use 'capsem resume <name>' to restore)
```

Note: suspend only makes sense for persistent VMs. If the VM is ephemeral, suspending would save state that gets destroyed on stop. capsem-sysutil should print a warning: "ephemeral VMs cannot be suspended, use 'capsem persist' first". The service enforces this -- rejects suspend for ephemeral VMs.

**Files:**
- `crates/capsem-agent/src/bin/capsem_sysutil.rs` -- suspend command dispatch (already scaffolded in T1)
- `crates/capsem-process/src/main.rs` -- handle SuspendRequest on port 5004
- `crates/capsem-service/src/main.rs` -- reject suspend for ephemeral VMs

### T8: CLI + MCP Tools

Add `capsem suspend` CLI command and MCP tools.

**CLI:**
```
capsem suspend <id>       # Suspend a running VM (quiesce + save state + stop)
capsem resume <name>      # Enhanced: warm restore if checkpoint exists, cold boot otherwise
```

`capsem resume` already exists for cold-boot of stopped persistent VMs. It's enhanced to detect a checkpoint file and do warm restore instead.

**MCP tools:**
- `capsem_suspend` -- suspend a running sandbox (persistent only)
- `capsem_resume` -- already exists, enhanced to handle warm restore

**Files:**
- `crates/capsem/src/main.rs` -- add Suspend command, enhance Resume to show warm/cold
- `crates/capsem-mcp/src/main.rs` -- add capsem_suspend tool

### T9: Testing Gate

**Unit tests (per sub-sprint):**
- Protocol roundtrip tests for all new message variants (T0)
- capsem-sysutil argv[0] dispatch tests (T1)
- VmHandle default impl tests (T3)
- Quiescence timeout logic (T4)

**Integration tests:**
- `capsem shell` -> type `shutdown` -> VM stops -> verify ephemeral session dir cleaned up
- `capsem create -n test` -> `capsem exec test shutdown` -> verify persistent session preserved
- `capsem create -n test` -> `capsem suspend test` -> verify checkpoint file exists -> `capsem resume test` -> verify running
- `capsem shell` -> verify `hostname` returns VM name/ID
- `capsem shell` -> verify `CAPSEM_VM_ID` and `CAPSEM_VM_NAME` env vars set

**Smoke (capsem-doctor additions):**
- Verify `/sbin/shutdown` symlink exists and points to capsem-sysutil
- Verify `hostname` returns non-default value
- Verify `CAPSEM_VM_ID` env var is set

**Files:**
- `tests/capsem-lifecycle/` -- new integration test directory
- `guest/artifacts/diagnostics/test_lifecycle.py` -- capsem-doctor additions

## What "Done" Looks Like

1. Inside a VM, `shutdown` stops it cleanly through the service (ephemeral destroyed, persistent preserved)
2. Inside a VM, `hostname` and `uname -n` return the VM name
3. `capsem suspend <name>` quiesces + saves state + stops (Apple VZ, ~730ms)
4. `capsem resume <name>` warm-restores from checkpoint (agent reconnects, filesystem unfreezes)
5. Inside a VM, `suspend` triggers the same suspend flow through the service
6. `reboot` inside the VM prints a clear "not supported" message
7. All flows use the service's existing stop/resume code paths -- no bypass
8. KVM returns clear "not supported" errors for checkpoint operations
9. `just test` passes, `capsem-doctor` passes with new lifecycle checks

## Open Questions (Resolved)

1. ~~Should guest shutdown destroy ephemeral VMs?~~ **Yes**, same as `capsem stop`.
2. ~~Shutdown timeout?~~ **5 seconds.**
3. ~~Should ShutdownRequest flow through service?~~ **Yes**, same code path as `capsem stop`.
4. ~~Dedicated vsock port for lifecycle?~~ **Yes, port 5004.** Clean separation from agent control channel.
5. ~~Override /usr/bin/uname?~~ **No.** `sethostname()` makes `uname -n` and `hostname` work natively.
