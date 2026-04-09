# Sprint: VM Lifecycle -- Tracker

## Phase 1: Shutdown + Identity

### T0: Protocol + Identity
- [x] Add `GuestToHost::ShutdownRequest` to `capsem-proto/src/lib.rs`
- [x] Add `GuestToHost::SuspendRequest` to `capsem-proto/src/lib.rs`
- [x] Add `GuestToHost::SnapshotReady` to `capsem-proto/src/lib.rs`
- [x] Add `HostToGuest::PrepareSnapshot` to `capsem-proto/src/lib.rs`
- [x] Add `HostToGuest::Unfreeze` to `capsem-proto/src/lib.rs`
- [x] Add `ProcessToService::ShutdownRequested { id }` to `capsem-proto/src/ipc.rs`
- [x] Add `ProcessToService::SuspendRequested { id }` to `capsem-proto/src/ipc.rs`
- [x] Add `ProcessToService::SnapshotReady { id }` to `capsem-proto/src/ipc.rs`
- [x] Add `ServiceToProcess::Suspend { checkpoint_path }` to `capsem-proto/src/ipc.rs`
- [x] Add `ServiceToProcess::PrepareSnapshot` to `capsem-proto/src/ipc.rs`
- [x] Add `ServiceToProcess::Unfreeze` to `capsem-proto/src/ipc.rs`
- [x] Roundtrip serde tests for all new variants
- [x] Service injects `--env CAPSEM_VM_ID={id}` when spawning capsem-process
- [x] Service injects `--env CAPSEM_VM_NAME={name_or_id}` when spawning capsem-process
- [x] Agent calls `sethostname(CAPSEM_VM_NAME)` after boot env applied
- [ ] Unit test: hostname set correctly (requires VM boot, deferred to T9)

### T1: capsem-sysutil Binary
- [x] Create `crates/capsem-agent/src/bin/capsem_sysutil.rs`
- [x] Add `[[bin]]` entry to `crates/capsem-agent/Cargo.toml`
- [x] Implement argv[0] dispatch (shutdown/halt/poweroff/reboot/suspend)
- [x] Implement vsock:5004 connect + send ShutdownRequest/SuspendRequest
- [x] Handle `shutdown -h now`, `shutdown -r` (error), `shutdown --help`
- [x] `reboot` prints "not supported in capsem sandbox", exit 1
- [ ] `suspend` on ephemeral VM: print warning, exit 1 (service-side enforcement, T7)
- [x] Unit tests for argv[0] parsing
- [x] Add capsem-sysutil to `_pack-initrd` in justfile
- [x] Add symlink creation in `guest/artifacts/capsem-init`:
  - `/sbin/shutdown -> /run/capsem-sysutil`
  - `/sbin/halt -> /run/capsem-sysutil`
  - `/sbin/poweroff -> /run/capsem-sysutil`
  - `/sbin/reboot -> /run/capsem-sysutil`
  - `/usr/local/bin/suspend -> /run/capsem-sysutil`

### T2: Shutdown Flow (End-to-End)
- [x] capsem-process: accept vsock connections on port 5004 (lifecycle)
- [x] capsem-process: handle GuestToHost::ShutdownRequest from port 5004
- [x] capsem-process: send ProcessToService::ShutdownRequested to service
- [x] capsem-process: trigger self-shutdown (same path as ServiceToProcess::Shutdown)
- [x] capsem-service: child reaper cleans up on process exit (ephemeral destroy, persistent preserve)
- [x] capsem-service: increase shutdown timeout from 150ms to 5s
- [x] capsem-agent: implement HostToGuest::Shutdown handler (sync + SIGTERM bash + 2s wait + break)
- [ ] Manual test: boot VM, type `shutdown`, verify clean stop
- [ ] Manual test: persistent VM shutdown -> `capsem resume` works

## Phase 2: Hypervisor + Quiescence + Reconnect

### T3: VmHandle Trait + Apple VZ Pause/Save
- [x] Add `pause()`, `resume()`, `save_state()`, `restore_state()`, `supports_checkpoint()` to VmHandle trait with default impls
- [x] Implement `AppleVzMachine::pause()` (main thread dispatch)
- [x] Implement `AppleVzMachine::resume()` (main thread dispatch)
- [x] Implement `AppleVzMachine::save_state(path)` (macOS 14+ guard)
- [x] Implement `AppleVzMachine::restore_state(path)` (macOS 14+ guard)
- [x] Wire through `AppleVzHandle` to machine methods
- [x] Unit test: supports_checkpoint() returns true on Apple VZ
- [x] Unit test: KVM default impls return errors

### T4: Quiescence Protocol
- [x] Agent: handle `PrepareSnapshot` (sync + fsfreeze -f / + send SnapshotReady)
- [x] Agent: handle `Unfreeze` (fsfreeze -u /)
- [x] Agent: error handling if fsfreeze fails (send Error, don't ack)
- [x] Process: quiescence helper -- send PrepareSnapshot, wait SnapshotReady (10s timeout), do op, send Unfreeze
- [x] Process: timeout handling -- if no SnapshotReady in 10s, send Unfreeze and abort
- [x] Unit test: quiescence timeout fires

### T5: Agent Reconnect
- [x] Agent: detect EOF on vsock fd (control or terminal)
- [x] Agent: reconnect loop with exponential backoff (100ms initial, 5s max, 30s total timeout)
- [x] Agent: re-send Ready after reconnect
- [x] Agent: fsfreeze -u / after reconnect if filesystem was frozen
- [x] Process: handle re-accepted vsock connections after restore
- [x] Process: re-run boot handshake (abbreviated) on reconnect

## Phase 3: Suspend/Resume End-to-End

### T6: Suspend/Resume Service Flow
- [x] Service: add `POST /suspend/{id}` endpoint
- [x] Service: reject suspend for ephemeral VMs (must persist first)
- [x] Service: send PrepareSnapshot to process, wait for SnapshotReady (handled by process internally)
- [x] Service: send Suspend { checkpoint_path } to process
- [x] Process: quiesce -> pause -> save_state -> stop sequence
- [x] Process: send StateChanged { state: "Suspended" }
- [x] Service: update InstanceInfo with checkpoint_path (handled via registry)
- [x] PersistentVmEntry: add `suspended: bool` and `checkpoint_path: Option<PathBuf>`
- [x] Resume: detect checkpoint file, pass --checkpoint-path to capsem-process
- [x] Process: --checkpoint-path flag -> boot VM, restore_state, resume (warm restore)
- [x] Process: after warm restore, wait for agent reconnect, send Unfreeze
- [ ] Manual test: suspend -> resume round-trip on Apple VZ

### T7: Guest-Initiated Suspend
- [x] Process: handle SuspendRequest on port 5004
- [x] Process: send ProcessToService::SuspendRequested to service
- [x] Service: handle SuspendRequested -> wait_for_exit handles checkpoint update
- [ ] Manual test: type `suspend` inside persistent VM -> VM suspends
- [ ] Manual test: `capsem resume <name>` after guest-initiated suspend

### T8: CLI + MCP Tools
- [x] CLI: add `capsem suspend <id>` command
- [x] CLI: enhance `capsem resume` to show warm/cold restore
- [x] MCP: add `capsem_suspend` tool to capsem-mcp
- [x] MCP: enhance `capsem_resume` tool description for warm restore
- [x] Unit test: CLI parse tests for suspend command

## Phase 4: Testing Gate

### T9: Testing Gate
- [ ] `just test` passes (all unit tests)
- [ ] Integration test: guest shutdown for ephemeral VM
- [ ] Integration test: guest shutdown for persistent VM + resume
- [ ] Integration test: hostname reflects VM name
- [ ] Integration test: CAPSEM_VM_ID/CAPSEM_VM_NAME env vars
- [ ] Integration test: suspend + warm resume (Apple VZ)
- [ ] capsem-doctor: verify /sbin/shutdown symlink exists
- [ ] capsem-doctor: verify hostname is non-default
- [ ] capsem-doctor: verify CAPSEM_VM_ID env var set
- [ ] CHANGELOG.md update
- [ ] Commit

## Notes
- **Dev assets symlink**: `_ensure-service` now symlinks `~/.capsem/assets -> repo assets/` so repacked initrd is used by the running service. Without this, dev builds and installed service use different initrds.
- **`tokio::task::spawn_blocking` unreliable for short-lived tasks**: The lifecycle handler never executed when using `spawn_blocking` -- the blocking thread pool didn't schedule it. Switched to `std::thread::spawn` which works immediately.
- **Interactive bash ignores SIGTERM**: The agent's Shutdown handler must SIGKILL after the 2s grace period. SIGTERM alone leaves bash alive, blocking the bridge loop and preventing process exit.
- **CLI `run_shell` must watch for VM exit**: Added `output_task` to `tokio::select!` so the shell exits when the IPC connection closes (VM shutdown/crash).
