# Sprint: virtio-blk overlay migration (closes loop-device-io-after-resume)

## TL;DR

Move the persistent overlay's `rootfs.img` off the loop-on-VirtioFS-on-APFS
sandwich and attach it to the guest as a real `virtio-blk` device. This
closes the `loop-device-io-after-resume` sprint and eliminates a class of
data-loss bugs that the closed-source Apple VZ VirtioFS implementation
makes structurally unfixable any other way.

A working spike is already on disk and proven against the production
failure mode. This sprint is "promote the spike to prod", not "design the
fix from scratch."

## Why now

The previous sprint (`sprints/loop-device-io-after-resume/`) shipped a
three-stage fsync, re-enabled the EXT4 journal, and added per-stage
suspend instrumentation. None of it closed the heavy-churn data loss.

Empirical root cause confirmed in 2026-05-03 forensic session:

- Pre-suspend: 250 churn files in `/tmp /var /opt /etc /usr/local` exist
  only in the guest kernel page cache for `/dev/loop0`. Host file is
  5.6MB, in-flight writes are in the 72MB `checkpoint.vzsave` memory
  capture.
- On resume: kernel writeback fires. Loop driver issues `REQ_NOWAIT`-
  flagged WRITE bios. Apple VZ's VirtioFS server returns EIO under
  concurrency. Failures hit the journal superblock at sector 2097152
  and the EXT4 superblock at sector 0. Result: `Aborting journal on
  device loop0-8` -> `Remounting filesystem read-only` -> 250/250
  files unreadable.

This isn't a code bug we can patch -- VirtioFS is a file-sharing
protocol being forced to handle block-device semantics. The closed-
source `virtiofsd` in Apple VZ doesn't honor `REQ_NOWAIT` back-pressure
the way real block storage does.

Rejected alternative #1: `fsfreeze /mnt/system`. Tried 2026-05-03,
broke 3 tests. Freezing the inner ext4 transitively freezes the
overlayfs upper, blocking every overlay write during boot/resume.

Rejected alternative #2: `vm.dirty_ratio` sysctl tuning. Reduces but
does not eliminate the resume-time burst. Costs IOPS in steady state.

Only structural fix: bypass VirtioFS for the block-device case.
virtio-blk speaks the contract Linux's block layer expects.

## The spike (on disk, working)

A minimal proof-of-concept is checked into the working tree (see "Spike
state" below). Two files modified, ~30 lines net:

1. **`crates/capsem-process/src/main.rs`** — passes
   `session_dir/guest/system/rootfs.img` as
   `BootOptions.scratch_disk_path`, which the existing `attach_disk`
   path attaches as a virtio-blk device. Appears in the guest as
   `/dev/vdb`.

2. **`guest/artifacts/capsem-init`** — when `/dev/vdb` is present
   (it always is, post-spike), use it as the overlay-upper device
   directly. Falls back to the old loop-on-VirtioFS path if absent
   (asserts the spike is wired before claiming a fix). The fallback
   should be DELETED in this sprint after the prod path is verified.

### What the spike proved (measured 2026-05-03)

End-to-end heavy-churn suspend/resume against the live service:

```
Pre-spike:   ok=0 fail=50  (cat /tmp/churn_<N> -> Input/output error)
Post-spike:  ok=250 fail=0 (every file reads cleanly)
dmesg ext4/loop0 errors:    pre=many, post=0
```

Boot path verified:

```
[0.106] virtio_blk virtio2: [vdb] 4194304 512-byte logical blocks (2.15 GB/2.00 GiB)
[0.203] EXT4-fs (vdb): mounted filesystem ... ordered data mode
```

APFS clonefile against the live virtio-blk-attached file (snapshot path):

```
clonefile OK
clone size=2147483648 blocks=15680
cmp: identical (byte-for-byte)
auto_snapshot ring 0..6 ran uninterrupted during the whole spike
```

So: snapshot path **already works**, no code change needed there.

## What this sprint must do (action items)

Land the spike + the cleanup that the spike intentionally skipped.
None of these are speculative — every one was either proven safe in
the spike or grep-trivial.

### Required to ship

1. **Promote the spike to a real change.** Drop the `// SPIKE:`
   comments. Rename `BootOptions.scratch_disk_path` ->
   `system_overlay_disk` (or `persistent_overlay_disk`); the field is
   no longer scratch-shaped. Update doc comments in
   `crates/capsem-core/src/lib.rs::create_virtiofs_session` (which
   today says "guest formats it as ext4 on first boot" via the loop
   device — it's now formatted via /dev/vdb).

2. **Universal application: ALL VMs get virtio-blk, ephemeral and
   persistent alike.** No fork in the boot path, no two test
   matrices. Today's "ephemeral can't suspend" was an accident of the
   loop-on-VirtioFS failure mode; the spike already runs against
   ephemeral VMs cleanly. Cost is ~zero (sparse 2GB file, 0 blocks
   allocated until written; deleted at session-dir cleanup).
   Side benefit: `capsem persist` (convert ephemeral -> persistent)
   becomes a registry flag flip with no storage migration.

3. **Delete the loop-on-VirtioFS fallback in `capsem-init`.** Once
   the production path is verified (see acceptance), there's no need
   to keep both. Today's spike keeps it as belt-and-suspenders;
   prod should keep one path.

4. **Run `tests/capsem-service/test_svc_fork.py` against the spike.**
   Fork uses APFS-cloned `system/` + `workspace/` + `session.db` as a
   reusable image. The clonefile mechanics are proven (see above);
   what's NOT proven yet is "boot a fresh VM from the cloned image
   with /dev/vdb attachment." Expected to pass — same plumbing as
   fresh VM — but actually run the test.

5. **Run `just smoke` end-to-end.** Catches anything that depended on
   `/dev/loop0` being the overlay device. There may be assumptions in
   `capsem-doctor` and friends.

6. **Grep + fix `/dev/loop0` assumptions.** Greppable in 5 minutes:

   ```bash
   rg --type-add 'capsem:*.{rs,sh,py,toml}' -t capsem 'loop0|/dev/loop' \
     crates/ guest/ tests/ src/
   ```

   Anything that asserts `/dev/loop0` is the system overlay needs to
   become `/dev/vdb` (or, better, query `/proc/mounts` for the device
   backing `/mnt/system`).

7. **Make the failing test pass.**
   `tests/capsem-service/test_svc_loop_device_after_resume.py`
   (committed in `b86e5fd` as the regression net) should flip from
   red to green. If it doesn't, the migration didn't fully close it.

8. **Commit + close the parent sprint.** Mark
   `sprints/loop-device-io-after-resume/ISSUE.md` resolved with a
   pointer to this sprint's commit. Move both sprint dirs into
   `sprints/done/`.

### Should-do followups (won't block ship but file as TODO)

9. **Decide what happens to `rootfs.img` on the VirtioFS share.**
   The file exists at `session_dir/guest/system/rootfs.img` because
   `create_virtiofs_session` creates it there, AND it's now also
   attached as virtio-blk by VZ. The host can read the file (when
   the VM is stopped, or via clonefile when running) but the guest
   never accesses it through VirtioFS anymore. Two options:
   - Leave it. Free, host can introspect when VM stopped.
   - Move it OUT of `guest/system/` and into the session_dir root
     (e.g. `session_dir/system-overlay.img`), so the VirtioFS share
     no longer references it. Cleaner; one source of truth.
   Recommend "leave it" for this sprint, "move it" for a followup.

10. **Block-mode (`STORAGE_MODE=block`) cleanup.** Today the
    `/dev/vdb` slot is also used by block mode for a SCRATCH disk.
    With this sprint, virtiofs mode also uses /dev/vdb. The legacy
    block mode is gated by the absence of `capsem.storage=virtiofs`
    on the kernel cmdline, which is set by default. Block mode is
    likely dead code; consider deleting in a followup.

11. **Field rename + docs.** `scratch_disk_path` is the wrong name
    for the persistent overlay. Already noted in #1.

## Acceptance criteria

The sprint is done when:

- [ ] Spike comments and fallback path are removed; production naming
      throughout.
- [ ] `tests/capsem-service/test_svc_loop_device_after_resume.py` is
      green.
- [ ] `tests/capsem-service/test_svc_resume_paths.py` is green
      (existing — regression check).
- [ ] `tests/capsem-service/test_svc_suspend_corruption.py` is green.
- [ ] `tests/capsem-service/test_svc_persistence.py` is green.
- [ ] `tests/capsem-lifecycle/test_vm_lifecycle.py` is green.
- [ ] `tests/capsem-service/test_svc_fork.py` is green.
- [ ] `just smoke` passes end-to-end.
- [ ] Heavy-churn manual repro from this ISSUE flips ok=0/fail=50 to
      ok=250/fail=0 against the live service.
- [ ] No `/dev/loop0` assumption remains in capsem-doctor or test
      fixtures (grep-clean).
- [ ] CHANGELOG entry written.
- [ ] `sprints/loop-device-io-after-resume/ISSUE.md` updated with the
      close-out commit hash and moved into `sprints/done/`.
- [ ] This sprint moved into `sprints/done/` after the above.

## Spike state (where to start in a new session)

The spike is **uncommitted** in the working tree at session start.
Files modified:

- `guest/artifacts/capsem-init` — `/dev/vdb` preferred, fallback to
  loop kept temporarily.
- `crates/capsem-process/src/main.rs` — `scratch_disk_path: Some(&system_img)`
  in the BootOptions construction.

These should be the starting point. Either:

- (a) Pick up the working-tree changes, run through the action items,
  then commit the whole result as one fix.
- (b) Commit the spike verbatim first ("spike: virtio-blk overlay
  proves heavy-churn is fixable"), then layer the cleanup commits on
  top. More commits, easier to revert any single one.

Recommendation: (b). Smaller commits = easier review.

## How to verify locally

```bash
# 1. Confirm spike is on disk (you should see both files modified)
git status

# 2. Repack initrd + sign + restart the live service
just _pack-initrd
codesign -s - --entitlements entitlements.plist --force --options runtime \
  target/debug/capsem-process target/debug/capsem-service
# Then restart the service manually (kill existing, re-spawn from target/debug)

# 3. Run the failing test (should be GREEN with spike applied)
CAPSEM_HOME="$(pwd)/target/test-home/.capsem" \
CAPSEM_RUN_DIR="$(pwd)/target/test-home/.capsem/run" \
CAPSEM_REQUIRE_ARTIFACTS=1 \
  uv run python -m pytest \
    tests/capsem-service/test_svc_loop_device_after_resume.py \
    tests/capsem-service/test_svc_resume_paths.py \
    tests/capsem-service/test_svc_suspend_corruption.py \
    tests/capsem-service/test_svc_persistence.py \
    tests/capsem-lifecycle/test_vm_lifecycle.py \
    tests/capsem-service/test_svc_fork.py \
    -v --tb=short

# 4. Heavy-churn manual repro via MCP (capsem MCP must be configured)
#    a. capsem_create name=verify
#    b. exec: tag=$(date +%N); for d in /tmp /var /opt /etc /usr/local; do
#         for i in $(seq 1 50); do echo "data-$tag-$i" > $d/churn_$RANDOM_$i;
#       done; done; sync
#    c. capsem_suspend id=verify
#    d. capsem_resume name=verify
#    e. exec: ok=0; fail=0; for f in /tmp/churn_* /etc/churn_* ...; do
#         if cat $f >/dev/null 2>&1; then ok=$((ok+1)); else fail=$((fail+1)); fi
#       done; echo "ok=$ok fail=$fail"
#    Expected: ok=250 fail=0
```

## Risk register (what could surprise us)

Listed in priority of likelihood-and-impact:

1. **`capsem fork` boot fails.** Untested in the spike. Mitigation:
   action item #4. Likelihood low (same plumbing as fresh VM),
   impact medium (fork is a real feature).

2. **A test fixture greps `/dev/loop0` and breaks.** Mitigation:
   action item #6. Likelihood medium, impact small (mechanical fix).

3. **KVM CI breaks.** virtio-blk attach already works on the KVM
   backend (we use it for the squashfs lower), but we haven't actually
   booted the spike under KVM. Mitigation: let CI tell us, fix
   in-place. Likelihood low, impact small.

4. **First-boot perf regression.** ~50-200ms `mke2fs /dev/vdb` on
   first boot of every VM (was previously only persistent VMs).
   Mitigation: irrelevant in the 5-10s VM cold-boot budget. Likelihood
   high, impact zero.

5. **Snapshot path race.** clonefile vs concurrent VZ writes.
   Mitigation: validated in spike (`cmp` returned identical), AND
   the EXT4 journal heals partial writes on next mount. Likelihood
   low, impact low.

## Out of scope

- Renaming `scratch_disk_path` to a more accurate name THROUGHOUT
  the codebase (only the BootOptions field needs renaming for
  acceptance; deeper rename is followup #11).
- Deleting block-mode entirely (followup #10).
- Moving rootfs.img out of the VirtioFS share entirely (followup #9).
- Reducing the 2GB sparse default for ephemeral VMs (could be 256MB
  to save inode pressure, but it's sparse so cost is zero).
- Anything observability-related — that's `sprints/observability-stop-the-bleeding/`.

## Related sprints / commits

- `sprints/loop-device-io-after-resume/ISSUE.md` — the parent sprint
  this closes. Documents what was tried (3-stage flush, journal,
  fsfreeze) and why each was insufficient. Read first.
- `sprints/observability-stop-the-bleeding/ISSUE.md` — sibling
  sprint. The W4 timing instrumentation it specifies is what made
  the forensic root-cause possible.
- Recent commits providing the foundation:
  - `7043dda fix(suspend): three-stage rootfs.img flush + don't claim Suspended on failure`
  - `e95229a fix(persistent-overlay): re-enable EXT4 journal on rootfs.img`
  - `867883d fix(service): guest-initiated shutdown -> Stopped, not Defunct`
  - `b86e5fd test(suspend): pin loop-device-io-after-resume bug with failing dmesg check`
    (the regression net that this sprint must flip green)
- The capsem-core hypervisor abstraction:
  - `crates/capsem-core/src/hypervisor/apple_vz/machine.rs::attach_disk`
    -- the function that already exists; we're feeding it a new path,
    not adding new attach machinery.
  - `crates/capsem-core/src/lib.rs::BootOptions` -- the struct that
    currently has `scratch_disk_path: Option<&Path>`.
