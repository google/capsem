# Spike: Checkpoint/Restore Feasibility

## Goal

Answer one question: **can we pause a running VM, save its state to disk, and restore it?** On both Apple VZ and KVM. This gates CP6 (Checkpoints + branch) in the next-gen plan.

## Success Criteria

| Platform | Pause/Resume | Save/Restore | VirtioFS survives | vsock survives |
|----------|-------------|-------------|-------------------|----------------|
| Apple VZ | works | works | yes/no | yes/no |
| KVM | works | works or ruled out | yes/no | yes/no |

We need hard answers, not "should work in theory." Each cell gets tested with a real VM.

## Non-Goals

- Production-quality code (this is throwaway spike code behind `#[cfg(test)]`)
- Checkpoint file format design
- Branch/fork semantics
- UI integration
- MCP tools

## Key Decisions Up Front

- Spike Apple VZ first -- it's the primary platform and the framework does the heavy lifting
- KVM spike is exploratory -- if native KVM checkpoint is too hard, evaluate "stop + reboot from filesystem" as fallback
- All spike code lives in test modules, not public API

---

## Phase 1: Apple VZ Pause/Resume (1 day)

### What

Add `pause()` and `resume()` to `AppleVzMachine`. Test that a running VM can be paused and resumed without losing state.

### Where

- `crates/capsem-core/src/hypervisor/apple_vz/machine.rs` -- add `pause()`, `resume()`
- `crates/capsem-core/src/hypervisor/apple_vz/mod.rs` -- wire through `AppleVzHandle`
- `crates/capsem-core/src/hypervisor/mod.rs` -- extend `VmHandle` trait

### State Machine Constraint

Apple VZ enforces strict state transitions. `pause()` is only valid from `Running`. `resume()` is only valid from `Paused`. Check `self.state()` before making the ObjC call and return a clear error if the precondition isn't met -- don't let the framework reject it with an opaque crash.

### Implementation Pattern

Identical to existing `start()` / `stop()`:

```rust
pub fn pause(&self) -> Result<()> {
    anyhow::ensure!(is_main_thread(), "must be called on main thread");
    anyhow::ensure!(
        self.state() == VmState::Running,
        "pause requires Running state, currently {:?}", self.state()
    );
    let (tx, rx) = std::sync::mpsc::channel();
    let completion = RcBlock::new(move |error: *mut NSError| {
        if error.is_null() {
            let _ = tx.send(Ok(()));
        } else {
            let desc = unsafe { format!("{:?}", (*error).debugDescription()) };
            let _ = tx.send(Err(anyhow::anyhow!("pause failed: {desc}")));
        }
    });
    unsafe { self.inner.pauseWithCompletionHandler(&completion); }
    spin_runloop_until(&rx).context("VM pause")
}
```

Resume is the same pattern with `resumeWithCompletionHandler`, guarded by `state() == Paused`.

### Test

```
boot VM -> run "echo before" via serial -> pause -> verify state == Paused
-> resume -> verify state == Running -> run "echo after" via serial -> stop
```

Also test the guard: calling `pause()` on an already-paused VM should return an error, not crash.

### Questions to Answer

1. Does `pauseWithCompletionHandler` exist in objc2-virtualization 0.3.2?
2. How fast is pause? (< 100ms expected)
3. Does the serial console still work after resume?
4. Does the guest kernel notice the pause? (check `dmesg` for time jump)

---

## Phase 2: Apple VZ Save/Restore (1-2 days)

### What

Add `save_state(path)` and `restore_state(path)` to `AppleVzMachine`. Test full checkpoint cycle.

### Where

Same files as Phase 1, plus checkpoint file I/O.

### State Machine Constraint

**Mandatory pause before save.** You cannot call `saveMachineStateTo:completionHandler:` on a Running VM -- it must be Paused first. `save_state()` must enforce this:

```rust
anyhow::ensure!(
    self.state() == VmState::Paused,
    "save requires Paused state (call pause() first), currently {:?}", self.state()
);
```

**Post-restore state.** After `restoreMachineStateFrom:completionHandler:` completes, the VM may land in Paused (not Running). The spike must verify what state the VM is in after restore and whether `resume()` is needed to get back to Running. Document the exact transition: `restore -> ? -> Running`.

### Implementation Pattern

```rust
pub fn save_state(&self, path: &Path) -> Result<()> {
    anyhow::ensure!(is_main_thread(), "must be called on main thread");
    anyhow::ensure!(
        self.state() == VmState::Paused,
        "save requires Paused state, currently {:?}", self.state()
    );
    let ns_path = NSString::from_str(path.to_str().context("path not UTF-8")?);
    // Use fileURLWithPath_isDirectory_ to avoid ambiguous URL resolution
    let url = unsafe { NSURL::fileURLWithPath_isDirectory(&ns_path, false) };
    let (tx, rx) = std::sync::mpsc::channel();
    let completion = RcBlock::new(move |error: *mut NSError| { /* same pattern */ });
    unsafe { self.inner.saveMachineStateToURL_completionHandler(&url, &completion); }
    spin_runloop_until(&rx).context("VM save")
}
```

**API name note:** The method in objc2-virtualization 0.3 bindings is likely `saveMachineStateToURL_completionHandler` (not `saveMachineStateTo_completionHandler`). Verify the exact selector name in the crate source before coding. Same for restore: `restoreMachineStateFromURL_completionHandler`.

**NSURL note:** Use `NSURL::fileURLWithPath_isDirectory_` (with explicit `false` for isDirectory) to avoid "invalid URL" errors from ambiguous path resolution. Local file paths passed to the framework must be proper file URLs, not bare paths.

### Test Sequence

```
boot VM -> write file "/tmp/marker" in guest
-> pause -> verify state == Paused
-> save state to ~/.capsem/checkpoints/test.vzstate
-> verify checkpoint file exists + measure size
-> stop VM
-> boot fresh VM with same config
-> restore state from checkpoint
-> observe post-restore state (Paused? Running?)
-> if Paused: call resume() -> verify Running
-> read "/tmp/marker" -> verify contents match
```

### Questions to Answer

1. Does `saveMachineStateToURL:completionHandler:` exist in the bindings? (macOS 14+, should be there). If not, fall back to raw `msg_send!`.
2. How big is the checkpoint file? (expect ~= allocated RAM)
3. How long does save take for 2GB / 4GB RAM?
4. **What state is the VM in after restore?** Paused? Running? Does it need an explicit `resume()`?
5. **Does VirtioFS survive restore?** (test: write file via VirtioFS before save, read after restore)
6. **Does vsock survive restore?** (test: open vsock connection before save, check after restore)
7. If vsock dies: is the CID (Context ID) stable after restore? If CID changes, agent routing breaks even with reconnection.
8. If vsock dies, how fast can we reconnect? (< 500ms = CP6 is still a Go with revised agent protocol)
9. **Stale file handles:** After restore, do open file descriptors in the guest get ESTALE from VirtioFS? (the host-side FUSE process may have restarted)

---

## Phase 3: Apple VZ Edge Cases (1 day)

### Tests

1. **Pause during VirtioFS I/O**: guest is actively writing to VirtioFS while pause fires. Does it corrupt? Does it resume cleanly? Specifically check for **ESTALE on open file handles** in the guest after restore -- if the host-side FUSE/VirtioFS process restarts, the guest may see stale handles.
2. **Save during network activity**: MITM proxy has open connections. What happens to them?
3. **Double pause**: pause an already-paused VM. Our guard should catch this, but verify the framework behavior if the guard is bypassed.
4. **Restore to wrong config**: restore checkpoint to a VM with different RAM/CPU. Should error -- verify it does.
5. **Checkpoint file portability (cross-process)**: save on one `capsem` process, restore on a completely fresh process (different PID, different boot). This validates that no ephemeral host-side state (open file descriptors for disk image, VirtioFS mounts) leaks into the checkpoint. **Critical for the daemon recovery model.**
6. **ENOSPC during save**: fill the target disk near capacity, then save. Does the framework leave a corrupted partial file that could confuse a later restore? If so, we need atomic-write semantics (save to temp, rename on success).
7. **Memory pressure during save**: save with host under memory pressure. Does it fail gracefully?

### Questions to Answer

1. What's the minimum macOS version? (likely 14.0 for save/restore, 12.0 for pause/resume)
2. **Entitlement requirements**: Beyond `com.apple.security.virtualization`, does saving to an arbitrary path require `com.apple.security.files.user-selected.read-write`? Or does writing to `~/.capsem/` (the app's own data directory) suffice? If sandboxed, the target directory may need pre-granted access.
3. Can we save to a file descriptor instead of a path? (for streaming to remote storage later)

---

## Phase 4: KVM Assessment (1-2 days)

### Timebox: 4 hours max on virtio ring issues

If native KVM checkpoint hits a wall with virtio ring inconsistencies, **stop debugging and pivot to Option B**. The virtio wall is where KVM snapshot spikes go to die. Don't sink time into it.

### Prior Art Research (first)

Before writing any code, check how these projects handle snapshot/restore:

- **Firecracker** (AWS): has snapshot/restore, open source, Rust. Check their virtio device serialization approach.
- **Cloud Hypervisor** (Intel): has live migration, Rust. Check their device state serialization.
- **crosvm** (Google): has suspend/resume. Check their approach.

If any of these have extractable device state serialization code, evaluate whether we can reuse it rather than building from scratch.

### Option A: Native KVM Checkpoint

Evaluate feasibility of serializing full VM state:

1. **vCPU state**: `KVM_GET_REGS`, `KVM_GET_SREGS`, `KVM_GET_FPU`, `KVM_GET_VCPU_EVENTS` -- straightforward, small data
2. **Guest memory**: `vm-memory` crate's `GuestMemoryMmap` -- can mmap directly to file, but it's the full RAM allocation
3. **GIC state** (aarch64): `KVM_DEV_ARM_VGIC_GRP_*` ioctls -- documented but fiddly
4. **Virtio device state**: console, block, vsock, virtio-fs queue state -- this is the hard part. Each device has descriptor rings, available/used rings, in-flight requests. **This is the virtio wall.**

### Spike Test (if Option A seems viable)

```
pause vCPU threads -> serialize vCPU regs -> dump guest memory to file
-> stop VM
-> boot new VM -> load guest memory from file -> restore vCPU regs -> resume
-> check if guest is alive
```

Skip virtio device state initially -- just test if the kernel resumes with memory + registers alone. If the kernel panics because virtio queues are gone, that tells us Option A requires full device state serialization (not just registers + memory).

### Option B: Filesystem-Only Checkpoint (fallback)

If native KVM checkpoint is too complex:

- "Checkpoint" = stop VM + preserve workspace filesystem state (`cp --reflink=auto` on btrfs/xfs, or plain `cp -a`)
- "Restore" = boot fresh VM + mount preserved filesystem
- "Branch" = reflink-copy the workspace, boot new VM pointing at clone
- No memory preservation, but branching still works for the primary use case (fork an environment)

This is 80% of the value for 10% of the effort. Most users want "branch my environment," not "freeze my CPU state."

### Questions to Answer

1. Is vCPU register save/restore enough to resume a paused kernel? (probably yes for simple cases)
2. Can we skip virtio device state and let the guest kernel re-probe devices? (probably no -- virtio queues will be in inconsistent state)
3. How much work is virtio device serialization? (estimate in days, not "a lot"). If > 5 days, pivot to Option B.
4. Does Firecracker/cloud-hypervisor have extractable device state serialization we can reuse?
5. Is Option B (filesystem-only) sufficient for the product? (branching works, but no "freeze in place" semantics)

---

## Phase 5: Decision (half day)

### Deliverable

A decision document at `tmp/next-gen/spike-checkpoint/results.md` with:

1. **Apple VZ verdict**: works / works with caveats / doesn't work
   - Pause/resume: yes/no + latency
   - Save/restore: yes/no + file size + latency
   - VirtioFS survives: yes/no
   - vsock survives: yes/no (if no: reconnection latency)
   - Minimum macOS version
2. **KVM verdict**: native checkpoint viable / filesystem-only fallback / not feasible
   - If native: estimated effort to productionize
   - If fallback: what the product loses (no freeze-in-place, cold restore only)
3. **Recommendation for CP6**: proceed as planned / modify scope / split further
4. **Trait design recommendation**: should `VmHandle` have `save`/`restore` or should it be a separate `Checkpointable` trait? (answer depends on whether both backends support it)

---

## Trait Extension (done during spike, kept if it works)

```rust
// Option A: extend VmHandle (if both backends support it)
pub trait VmHandle: Send {
    fn stop(&self) -> Result<()>;
    fn state(&self) -> VmState;
    fn serial(&self) -> &dyn SerialConsole;
    fn as_any(&self) -> &dyn std::any::Any;

    // New -- default to unsupported
    fn pause(&self) -> Result<()> { Err(anyhow::anyhow!("pause not supported")) }
    fn resume(&self) -> Result<()> { Err(anyhow::anyhow!("resume not supported")) }
    fn save_state(&self, _path: &std::path::Path) -> Result<()> { Err(anyhow::anyhow!("save not supported")) }
    fn restore_state(&self, _path: &std::path::Path) -> Result<()> { Err(anyhow::anyhow!("restore not supported")) }
    fn supports_checkpoint(&self) -> bool { false }
}

// Option B: separate trait (if only Apple VZ supports it)
pub trait Checkpointable {
    fn pause(&self) -> Result<()>;
    fn resume(&self) -> Result<()>;
    fn save_state(&self, path: &std::path::Path) -> Result<()>;
    fn restore_state(&self, path: &std::path::Path) -> Result<()>;
}
```

Decision made in Phase 5 based on KVM results.

---

## Timeline

| Phase | Days | Blocker |
|-------|------|---------|
| Phase 1: Apple VZ pause/resume | 1 | None |
| Phase 2: Apple VZ save/restore | 1-2 | Phase 1 |
| Phase 3: Apple VZ edge cases | 1 | Phase 2 |
| Phase 4: KVM assessment | 1-2 | None (parallel with Phase 3) |
| Phase 5: Decision | 0.5 | Phases 3+4 |
| **Total** | **4-6 days** | |

Phase 4 (KVM) can run in parallel with Phase 3 (Apple VZ edge cases) if there's a Linux machine available.

## Risk Mitigations

1. **objc2-virtualization doesn't expose save/restore methods**: Check the crate docs and source before writing code. If missing, use raw `msg_send!` to call the ObjC selector directly. The underlying framework method exists in macOS 14+.
2. **vsock doesn't survive restore**: Design the agent reconnection protocol during the spike. The agent already handles vsock disconnection (it's just a TCP-like stream) -- measure how fast it reconnects.
3. **KVM native checkpoint is too hard**: Option B (filesystem-only) is a viable product. Most users want "branch my environment" not "freeze my CPU state." Document the tradeoff clearly.
4. **Checkpoint files are huge**: Test with 2GB RAM (the planned default for secondary VMs). If file size is a problem, investigate incremental/dirty-page checkpoints (Apple VZ may not support this, but KVM could via `KVM_GET_DIRTY_LOG`).
