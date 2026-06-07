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

## Fix: serialize save/restore in capsem-service

`capsem-service` holds a single `tokio::sync::Mutex` across **every**
`handle_suspend` and `handle_resume` call. The lock is acquired at
the top of each handler and held until:

- For suspend: the per-VM `capsem-process` has exited, meaning the
  checkpoint file is durable.
- For resume: the new `capsem-process` has signalled
  `.ready` (boot through `restoreMachineStateFromURL` has returned).

Concurrent clients still see their requests succeed; they just queue
behind the in-flight save/restore. The lock is per-service, so in
production (one `capsem-service` per host per user) this fully
serializes VZ save/restore on that host.

See `crates/capsem-service/src/main.rs`
(`ServiceState::save_restore_lock`).

## Tests

`tests/capsem-mcp/test_stress_suspend_resume.py` must run serially
(`-n 1` under pytest-xdist, or without xdist). Running the stress
harness at `-n 2` or higher creates **multiple `capsem-service`
processes** (one per xdist worker). The in-service lock does not span
services, so each worker's service can race another worker's. That's
an artificial scenario -- a deployed host runs exactly one service --
but the test cannot observe the fix under concurrency. Stick to
`-n 1` for correctness measurement.

## Related past bugs

- `sprints/vsock-resume-reconnect/` -- vsock half-open + VZ path
  canonicalization. Closed earlier modes, left the loop-device tail.
- `sprints/loop-device-io-after-resume/` -- this gotcha's sprint home.
