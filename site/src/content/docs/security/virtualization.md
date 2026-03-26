---
title: Virtualization Security
description: VirtioFS sandboxing, resource limits, and hypervisor hardening.
sidebar:
  order: 5
---

The hypervisor layer isolates guest VMs from the host using hardware virtualization (Apple VZ on macOS, KVM on Linux). This page covers the security properties of the VirtioFS shared filesystem -- the primary guest-to-host data channel beyond vsock.

## VirtioFS Threat Model

VirtioFS exposes a POSIX-compatible shared mount between host and guest. The guest's workspace (`/root`) is backed by a host directory (`~/.capsem/sessions/<id>/workspace/`).

| Component | Trust | Implication |
|-----------|-------|-------------|
| Guest FUSE client | Untrusted | May send malformed requests, attempt path traversal, exhaust resources |
| Host VirtioFS server | Trusted | Must validate all requests, enforce limits, never trust guest input |
| Shared directory | Host-controlled | Capsem creates it fresh per session; no external process modifies it |

## Path Traversal Protection

Every FUSE LOOKUP resolves a single path component (filename) under a parent inode. The host validates names and paths at two levels:

**Name validation** (rejects before any filesystem access):
- Empty strings
- `.` and `..`
- Names containing `/` or `\0`

**Path canonicalization** (resolves symlinks, verifies containment):
```
child_path = parent_path.join(name)
canonical  = child_path.canonicalize()
assert canonical.starts_with(root_canonical)
```

If the guest creates a symlink inside the workspace pointing outside it (e.g., `ln -s /etc/passwd escape`), `canonicalize()` follows the symlink and the containment check rejects the resolved path.

### TOCTOU Analysis

There is a time-of-check-to-time-of-use window between `canonicalize()` and the subsequent filesystem operation. Exploiting this window requires a host-side process to replace a directory in the workspace with a symlink between the two syscalls.

This is acceptable because:
1. The untrusted party is the **guest**, not host processes. The guest communicates via FUSE opcodes and cannot manipulate the host filesystem directly.
2. The shared directory is **Capsem-controlled** (`~/.capsem/sessions/<id>/workspace/`). No external process should modify it during a VM session.
3. A malicious host process already has the same user privileges as Capsem and could attack directly.

If defense-in-depth against compromised host processes is ever needed, the implementation can migrate to fd-relative operations (`openat` with `O_NOFOLLOW` + `O_PATH`).

## Resource Exhaustion Defenses

A malicious guest can attempt to exhaust host resources via the FUSE protocol. The VirtioFS server enforces hard limits at every level:

| Attack Vector | Defense | Limit |
|---------------|---------|-------|
| Giant read request (`FUSE_READ` with `size = 4GB`) | Clamp to `max_read` from FUSE_INIT | 1 MB |
| Oversized descriptor chain | Reject in `gather_readable` | 2 MB total |
| File descriptor exhaustion (open millions of files) | `FileHandleTable` capacity limit | 4096 handles |
| Unbounded descriptor accumulation | Per-descriptor and total size checks | Enforced per request |

All limits are enforced host-side. Guest-negotiated values (e.g., `max_read` in FUSE_INIT) are treated as upper bounds, not trusted inputs.

## Data Integrity

The VirtioFS server propagates all I/O errors to the guest:

- **`fsync`/`fsyncdir`**: Sync errors are mapped to FUSE errno and returned. The guest learns if data durability failed.
- **`flush`**: Flush errors are returned as FUSE errors, not silently dropped.
- **Invalid file handles**: Return `EBADF` instead of silently succeeding.

This ensures the guest kernel marks pages correctly and applications can detect write failures.

## Async I/O Isolation

FUSE request processing runs on a **dedicated worker thread**, not on the vCPU thread. This prevents a slow host disk from freezing the guest CPU.

```
Guest vCPU thread          Worker thread
    |                          |
    |-- MMIO write (notify) -->|
    |   (returns immediately)  |
    |                          |-- process FUSE request
    |                          |-- host disk I/O
    |                          |-- write used ring
    |   <-- irqfd interrupt ---|
    |                          |
```

The vCPU thread sends a queue index over a channel and returns immediately. The worker processes the request, writes the response to the virtio used ring, and injects an interrupt into the guest via `irqfd`. Memory barriers (`Acquire`/`Release` fences) in the virtqueue ensure correct ordering between threads.

## Memory Safety

All FUSE struct deserialization uses a safe `read_struct<T>` function that returns `Option<T>`:
- Hard bounds check (`buf.len() < size_of::<T>()`) in all builds (not just debug)
- Returns `None` on short buffers -- callers map this to a FUSE error response
- No `unsafe` in the public API; the internal `read_unaligned` is encapsulated behind the bounds check

Every FUSE handler validates its input body before proceeding. Malformed requests result in clean error responses, not undefined behavior.
