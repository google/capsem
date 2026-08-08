# Sprint: loop-device I/O errors on persistent overlay after resume

## Status: CLOSED

Resolved by `sprints/done/virtio-blk-overlay-migration/` (the
"prove-it-and-promote" sprint). Root cause was structural -- Apple
VZ's closed-source virtiofsd EIOs the loop driver under writeback
pressure on resume, no amount of fsync/journal/freeze tuning closes
it. The fix was to bypass VirtioFS for the system overlay entirely
and attach `rootfs.img` as a real virtio-blk device (`/dev/vdb`).

The acceptance regression test (`b86e5fd`) is now green; heavy-churn
manual repro flips ok=0/fail=50 -> ok=250/fail=0; dmesg stays clean
of `loop0` errors after suspend/resume.

The same close-out commit also fixed three resume-stability
adjacent bugs that surfaced once the loop-device EIO was gone:
fast-fail on capsem-process death during resume, retryable
`UnexpectedEof` during post-restoreState handshake, and a control-
bridge replay buffer so in-flight `HostToGuest` commands aren't
dropped when Apple VZ kills the vsock mid-write.

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

## Update 2026-05-03: what's been tried (and what's left)

**Tried and shipped (commit `7043dda`):**
- Three-stage flush: guest `sync()` + `BLKFLSBUF` + `fsync(/dev/loop0)`
  + guest `fsync(/mnt/shared/system/rootfs.img)` (sends FUSE_FSYNC over
  VirtioFS) + host `sync_all()` on rootfs.img after `save_state`.
- Result: closes the user-visible `cd /root && ls` failure (the cwd
  inode is no longer stale on resume). Simple suspend/resume is fully
  fixed; the standard test suite (33 tests across capsem-lifecycle/,
  capsem-service/test_svc_persistence.py, test_svc_resume_paths.py,
  test_svc_suspend_corruption.py) is green.

**Tried 2026-05-03 (this sprint, additional):**
- Re-enabled the EXT4 journal on the persistent overlay
  (`mke2fs ... ` with default `has_journal`, mount `data=ordered`).
  Confirmed via `tune2fs -l /dev/loop0`:
  `Filesystem features: has_journal ... metadata_csum`.
- Result: journal does its job (`needs_recovery` flag flips on
  remount and metadata replays), but the heavy-churn case STILL
  reproduces:
  - `ls /tmp/churn_*` works (directory metadata is consistent post-
    journal-replay).
  - `cat /tmp/churn_<N>` returns `Input/output error` for some files.
  - dmesg still emits `EXT4-fs (loop0): failed to convert unwritten
    extents -- potential data loss! (inode N, error -5)` and matching
    `I/O error, dev loop0, sector N op 0x1:(WRITE) flags 0x800`.
- **Conclusion:** the journal recovers METADATA but cannot conjure
  DATA blocks the loop device refused to write. The flag `0x800`
  on the failing WRITE bios indicates REQ_NOWAIT (or similar in
  6.6) — VirtioFS is rejecting writes that would block. The problem
  is NOT in EXT4; it's that VirtioFS can't keep up with the loop
  device's write queue under churn, and Apple VZ's VirtioFS doesn't
  honor the back-pressure contract a normal block device would.

**Left to try (ordered by smallest diff first):**

1. **Move `rootfs.img` off VirtioFS to a real VZ block device.**
   This is the ONLY remaining path that eliminates the divergence
   by construction: Apple VZ owns the file, no VirtioFS in the
   middle, no can't-keep-up-with-writes failure mode. Use
   `VZDiskImageStorageDeviceAttachment` with
   `cachingMode=Uncached, synchronizationMode=Full`. Workspace
   stays on VirtioFS (the user-visible host-readable share);
   only the system overlay backing moves.

   Drawbacks (already documented in the prior session's analysis):
   - Host can no longer read overlay contents while VM runs (today
     it can `ls ~/.capsem/run/persistent/<vm>/system/upper/`).
   - Snapshot path needs to handle a separate file-level clone of
     `system-rootfs.img` (today's whole-tree APFS clonefile won't
     work for an open VZ disk).
   - Boot path forks for persistent vs ephemeral (ephemeral can
     keep using VirtioFS since it never suspends).

2. **If (1) is too invasive in one shot, mount the loop device
   with `sync` and accept the perf hit.** Forces every write to
   wait for the underlying file commit before the bio completes;
   eliminates the unwritten-extent backlog entirely. Big IOPS hit
   (each write is a synchronous round-trip through VirtioFS) but
   guaranteed to close the symptom. Useful as a temporary
   correctness band-aid while (1) is being designed.

3. **Last resort: drop the loop device entirely** and accept that
   persistent overlay-upper has to live on tmpfs (lost on every
   stop) until (1) lands. This is a regression for users.

The dmesg-failing test
(`tests/capsem-service/test_svc_loop_device_after_resume.py`,
commit `b86e5fd`) is intentionally still red and will stay red
until (1) lands. It's the regression net for this sprint.

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
