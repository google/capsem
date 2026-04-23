# Sprint: loop-device I/O errors on persistent overlay after resume

## TL;DR

After `restore_state` succeeds, on ~8% of persistent-VM resume cycles
the guest kernel comes back up with the EXT4 overlay-upper (backed by
a loop device) in a hard I/O error state. The guest agent detects the
broken control channel, tries to re-handshake, and the host reports
`vsock failed: initial handshake failed: BootReady read failed: failed
to fill whole buffer` because the guest has already wedged at the
kernel level before it can send `BootReady`. VZ is happy with the
restore — this is not a VZ / vsock bug, it's a kernel-level
disk-backing issue.

Spun out of `sprints/vsock-resume-reconnect/` after that sprint fixed
the VZ path-canonicalization and vsock half-open bugs but the 4/50
loop-device tail persisted.

## Fingerprint

Host `process.log`:
```
ERROR vsock failed: initial handshake failed: BootReady read failed: failed to fill whole buffer
```

Guest `serial.log`:
```
[capsem-agent] control channel error: Connection reset by peer (os error 104)
loop: Write error at byte offset 20058112, length 4096.
I/O error, dev loop0, sector 274432 op 0x1:(WRITE) flags 0x0 phys_seg 1 prio class 2
loop: Write error at byte offset 20582400, length 4096.
loop: Write error at byte offset 21106688, length 4096.
loop: Write error at byte offset 21630976, length 4096.
EXT4-fs (loop0): failed to convert unwritten extents to written extents -- potential data loss!  (inode 72, error -5)
Buffer I/O error on device loop0, logical block 34304
```

`fsfreeze` on the overlay is ALSO reporting errors during the pre-save
quiescence path — from an earlier artifact:
```
fsfreeze: I/O error, dev loop0, sector 274: unfreeze failed: Invalid argument
```

So the overlay is already in a degraded state *at save time* on these
runs, not just at resume.

## Current measurements (commit `03cf3f4` HEAD)

- `test_stress_suspend_resume.py` at `-n 8`, 50 iters: **46/50 pass.**
- All 4 failures show the signature above.
- No `permission denied` / `VZErrorDomain Code=12` anymore — the
  canonicalize fix closed that mode.
- No `BrokenPipe` / `ConnectionReset` at the handshake write — the
  vsock hot-swap closed that too.
- The whole 8% tail is this loop-device issue.

## Suspected cause (not yet proven)

The persistent VM's writable overlay is an EXT4 filesystem on a loop
device, with the backing `.img` on the host at
`~/.capsem/run/persistent/<id>/rootfs.img` (or the equivalent canonical
path under `/private/var/folders/.../persistent/...` in tests).

Hypothesis: writes buffered in the guest kernel's page cache at
`save_state` time are NOT all flushed to the backing `.img` before the
VM is paused. Apple VZ captures the guest's in-memory block-cache
state in the snapshot, but the on-disk `.img` is behind. On resume,
the kernel tries to issue the pending writes against what it believes
is an up-to-date backing file, but the underlying storage layer
(virtio-blk? the macOS file system?) rejects them as stale/invalid,
producing the cascading I/O errors.

Evidence for:
- `fsfreeze` itself is failing on the overlay before save. If the
  freeze is half-applied, writes could be sneaking through right up
  until save_state fires.
- Errors cluster in an arithmetic progression (offsets 20058112,
  20582400, 21106688, 21630976 — step 524288 = 512KiB =
  128 blocks). Consistent with a dirty writeback batch.
- Happens only on persistent VMs (which have a loop-backed overlay).
  Ephemeral VMs use tmpfs for the upper, so there's nothing to flush.

Evidence against (or alternate theories worth checking):
- Could be an Apple VZ bug in how it serializes virtio-blk state.
- Could be a `sparse file` + `fsync` race on macOS's APFS.

## Things already ruled out

- Not VZ path canonicalization — fixed in `03cf3f4`, failures
  continued.
- Not vsock half-open / framing desync — fixed in `60b57a1`, failures
  continued.
- Not `handle_suspend` returning before child exit — fixed in
  `03cf3f4`, failures continued.
- Not insufficient `fsync` on the `.vzsave` file — that was fixed by
  commit `3ccdce9` earlier, which explicitly `fsync`s the vzsave
  after `saveMachineStateToURL`. The checkpoint file is durable; it's
  the rootfs.img that's suspect.

## Where to start in a new session

1. **Check whether `fsfreeze` is actually freezing.** Inspect
   `crates/capsem-agent/src/main.rs` where the guest runs
   `fsfreeze -f /` before sending `SnapshotReady`. Is the exit code
   checked? Is the error `"Invalid argument"` being swallowed? If
   fsfreeze is failing, the "quiescence" isn't quiescing anything.
2. **Add an explicit host-side `fsync` on the rootfs.img backing file
   during suspend.** The suspend handler in
   `crates/capsem-process/src/vsock.rs` runs `fsync(.vzsave)` after
   `save_state`. Add a parallel `fsync` on the persistent overlay's
   backing file. If the hypothesis is right, this should close the
   loop-cache/.img skew.
3. **Try running without fsfreeze** (disable it entirely) and see if
   failures persist. Would isolate "freeze is half-broken" from
   "persistence model is fundamentally unsafe".
4. **Check whether the `.img` is opened with `O_DSYNC` / `O_SYNC`** in
   the VZ block-device attachment. If VZ is using buffered I/O and we
   don't flush, page cache divergence is a natural consequence.
5. **Look at what `FileHandle.isReadOnly` / `cachingMode` is on the
   VZDiskImageStorageDeviceAttachment** used for the persistent
   overlay (`crates/capsem-core/src/hypervisor/apple_vz/`).
   `VZDiskImageCachingMode.uncached` might be required.

## Repro

```bash
CAPSEM_STRESS=1 uv run pytest tests/capsem-mcp/test_stress_suspend_resume.py \
    -n 8 --tb=line -q
```

Expected: 46/50 today. Goal: 50/50.

## Scope

**In scope:**
- Diagnose and fix the loop-device I/O divergence.
- Add a single regression test (unit or VM-integration) that
  specifically exercises the persistent-overlay flush path.
- Move the stress-harness gate closer to 50/50.

**Out of scope:**
- Redesigning the persistence model (loop-backed EXT4 → VirtioFS,
  etc.). That's a bigger architectural change and should happen only
  if no reasonable flush/caching fix exists.
- Anything vsock-layer — already closed out in the sibling sprint.

## Non-goals

- Do not widen the handshake retry classification to include
  `UnexpectedEof`. That was explicitly rejected in the vsock sprint —
  retrying against a guest wedged on a kernel I/O failure burns the
  30s readiness budget without progress.
