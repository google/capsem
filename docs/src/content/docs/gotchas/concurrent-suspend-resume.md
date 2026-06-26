---
title: Concurrent suspend/resume corrupts VirtioFS overlay
description: Apple's Virtualization.framework does not tolerate overlapping save_state / restore_state calls on sibling VMs on the same host.
sidebar:
  order: 1
---

## Symptom

A persistent VM resumes "successfully" but becomes unusable: the guest
kernel logs an avalanche of I/O errors against `/dev/loop0`, the EXT4
overlay goes hard-fail, the agent's control vsock dies, and host-side
logs show:

```
ERROR vsock failed: initial handshake failed: BootReady read failed: failed to fill whole buffer
```

Guest `serial.log` shows the actual failure:

```
loop: Write error at byte offset 1204224, length 4096.
I/O error, dev loop0, sector 2352 op 0x1:(WRITE) ...
EXT4-fs (loop0): failed to convert unwritten extents to written extents -- potential data loss!
```

Write errors cluster at a 512 KiB stride (offsets 1204224, 1728512,
2252800, ...) -- one dirty-writeback batch per error -- and sometimes
include a single deep-offset write.

## Root cause

Apple's Virtualization.framework does not tolerate concurrent VZ
lifecycle operations on sibling VMs. If VM A is mid
`saveMachineStateToURL` while VM B is calling
`restoreMachineStateFromURL`, terminating, or spawning a fresh VM, one
of them can come back with the VirtioFS-backed overlay image in a
state the restored guest can't make sense of. The VirtioFS ring state
captured inside the vzsave ends up referencing FUSE descriptors the
host has already torn down or re-keyed on behalf of the sibling VM.

This is a host-level (macOS kernel + VZ framework) concurrency
interaction. It is not caused by our guest code, the agent's
`sync + BLKFLSBUF + fsync(/dev/loop0)` quiescence, or anything in the
Rust host code paths.

## Fix: serialize Apple VZ lifecycle in capsem-service

`capsem-service` holds a single in-process `tokio::sync::RwLock` plus a
host-wide flock across Apple VZ lifecycle edges. Cold provision/start and
stop/delete teardown take shared/read guards; suspend and resume take
exclusive/write guards. The guard is acquired before the service spawns or
signals `capsem-process` and is held until:

- For suspend: the per-VM `capsem-process` has exited, meaning the
  checkpoint file is durable.
- For resume: the new `capsem-process` has signalled
  `.ready` (boot through `restoreMachineStateFromURL` has returned).
- For provision/start: the new `capsem-process` has signalled `.ready`
  (boot through `startWithCompletionHandler` has returned).
- For stop/delete: the `capsem-process` has exited and VZ teardown has
  completed.

Concurrent clients still see their requests succeed. Independent cold starts
can overlap, but checkpoint save/restore remains exclusive and teardown cannot
cross a checkpoint edge. The in-process `RwLock` orders VMs managed by one
service, and the host-wide flock at
`/tmp/capsem-vz-save-restore-<uid>.lock` extends the same ordering across
pytest-xdist workers or any other sibling `capsem-service` process owned by
the same user.

See `crates/capsem-service/src/main.rs`
(`ServiceState::save_restore_lock`) and
`crates/capsem-service/src/startup.rs` (`VzHostLock`).

## Tests

`just test` intentionally runs Python integration tests under
`pytest -n 4 --dist=loadfile`. That creates multiple service processes, so
the host-wide flock is required test and product infrastructure. Do not
demote suspend/resume, lifecycle, or provisioning tests to `-n 1` to avoid
this class of failure; a concurrent VZ lifecycle failure means the shared
rail regressed.

Timing and benchmark probes are different: their assertion is the measured
number. `just test` runs the non-serial integration canary first, then runs
`tests/capsem-serial/` alone so boot and lifecycle numbers measure Capsem
rather than a sibling benchmark stealing the same VZ launch budget.

## Related past bugs

- `sprints/vsock-resume-reconnect/` -- vsock half-open + VZ path
  canonicalization. Closed earlier modes, left the loop-device tail.
- `sprints/loop-device-io-after-resume/` -- this gotcha's sprint home.
