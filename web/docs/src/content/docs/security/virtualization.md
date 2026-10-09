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
| Shared directory | Guest-writable through VirtioFS | Every host access must remain beneath the already-open workspace descriptor |

## Path Traversal Protection

Every FUSE LOOKUP resolves a single path component (filename) under a parent inode. The host validates names and paths at two levels:

**Name validation** (rejects before any filesystem access):
- Empty strings
- `.` and `..`
- Names containing `/` or `\0`

**Descriptor-relative access** opens the workspace once, descends one component
at a time with `openat`, and uses `O_NOFOLLOW` in the same syscall that opens
an entry. Directory descriptors, rather than reconstructed absolute paths,
anchor later reads, writes, links, renames, metadata operations, and deletion.
An entry swapped for a symlink between requests cannot redirect a later open.

The FUSE inode table also canonicalizes identities needed for guest symlink
semantics and checkpoint restore, but canonicalization is not the authority for
host filesystem syscalls. Guest-writable filesystem access goes through
`capsem_foundation::unix::contained::ContainedDir` or the equivalent
descriptor-backed VirtioFS operation. A symlink to `/etc/passwd`, FIFO, socket,
or device is reported as such and is never followed as a regular file.

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

## Workspace Ownership and Permissions

The workspace is not a place to separate guest users from each other. Capsem's boundary is the VM: everything inside the guest is the same untrusted party. Unix permissions in `/root` are not a security boundary inside the guest, and on macOS they do not separate users at all.

The guest kernel mounts VirtioFS with `default_permissions`, so it makes every permission decision itself from the owner and mode the server reports. (`/proc/mounts` does not show the option: virtio-fs does not print FUSE mount options.) What those decisions mean depends on what the host server reports as the owner.

| Host | Owner the server reports | Effect inside the guest |
|------|--------------------------|-------------------------|
| macOS (Apple VZ) | The uid and gid of the caller, taken from each request | Every caller is the owner of every entry, so the owner's mode bits apply to everyone. Any guest uid can read, write, truncate, unlink and create wherever the owner could. |
| Linux (KVM) | `0:0` for every entry | Permissions apply as if root owns the whole workspace. |

Neither server changes ownership on the host: a guest `chown` succeeds and has no effect, and every file in the host directory belongs to the user running Capsem.

On Apple VZ, "every entry is owned by root" is only root's view. The same file, re-read with fresh attributes, reports a different owner to each caller:

| Caller | Owner reported for one `0640` file |
|--------|-------------------------------------|
| root | `0:0` |
| uid 1000 | `1000:1000` |
| uid 2345 | `2345:2345` |

Attributes are cached per inode, not per caller, so a listing can show the owner from whoever refreshed the entry last. A uid with every capability dropped (`setpriv --inh-caps=-all --bounding-set=-all --no-new-privs`) appends to, truncates and unlinks a file root created with mode `0640`, and creates files in a root-owned `0755` directory. The same writes on the guest's `tmpfs` are refused.

The container workload runs in a user namespace whose root maps to VM uid 100000. The guest kernel sends the caller's ids as the caller's own user namespace sees them (kernel patch `0002`, see [Kernel Hardening](/security/kernel-hardening/)), so on Apple VZ the container's root is reported as uid 0 on disk and owns the workspace through the mount's idmap, the same way the VM's root does.

To reproduce on macOS, in a VM: as root, `echo x > /root/f; chmod 640 /root/f`, then as another uid with fresh attributes (`sync; echo 3 > /proc/sys/vm/drop_caches`), `stat -c '%u:%g' /root/f` and `echo y >> /root/f`.

## KVM Warm-Checkpoint Integrity

On Linux, saving guest RAM and virtqueue indices is not sufficient for a warm
restore: the guest also retains VirtioFS inode numbers and open-handle IDs. The
KVM checkpoint therefore captures the host VirtioFS processor only after its
worker has drained both request queues, then restores that state before any
queue is activated.

The backend payload is versioned and bounded. Paths are stored relative to the
current share root as raw Unix bytes, while inode and handle records carry host
filesystem identity. Restore fails closed if a path is missing or replaced, a
restored access path follows a symlink outside the share, an open object cannot
be reconstructed, the share tag or read-only policy changed, or the MMIO slot
topology is inconsistent.
Absolute host roots are never accepted from the checkpoint as authority.

Apple VZ continues to use the platform's native machine-state restore path;
the KVM-specific format does not replace or weaken that behavior.

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
