---
title: Hypervisor Architecture
description: How Capsem abstracts VM management across macOS, Linux, ChromeOS, and Windows using platform-native hypervisors.
sidebar:
  order: 1
---

Capsem runs AI agents inside sandboxed Linux VMs. The hypervisor layer abstracts VM lifecycle, device management, and guest-host communication across four platforms.

## Supported Platforms

| Platform | Hypervisor | VMM | Status |
|----------|-----------|-----|--------|
| macOS (Apple Silicon) | Virtualization.framework | Embedded (Apple VZ) | Production |
| Linux (aarch64 / x86_64) | KVM | Embedded (rust-vmm) | Production |
| ChromeOS | KVM | Embedded (nested) or vm_concierge | Planned |
| Windows (x86_64) | Hyper-V (WHPX) | crosvm (subprocess) | Planned |

The guest VM is always Linux (Debian bookworm, custom hardened kernel). Only the host-side VMM changes per platform.

## Architecture

On macOS and Linux, the VMM is **embedded** in the Capsem binary -- no external process needed. On Windows, crosvm runs as a subprocess.

```mermaid
graph TD
    A[capsem-app] --> B[capsem-core]
    B --> C{Hypervisor Trait}
    C -->|macOS| D["Apple VZ (embedded)"]
    C -->|Linux| E["KVM (embedded, rust-vmm)"]
    C -->|ChromeOS| H["KVM nested / vm_concierge"]
    C -->|Windows| F["crosvm (subprocess, WHPX)"]
    D --> G[Linux Guest VM]
    E --> G
    H --> G
    F --> G
```

**Embedded VMM (macOS, Linux):** All VM management logic runs in-process. On macOS, Apple's Virtualization.framework provides the API. On Linux, the `kvm-ioctls` crate talks directly to `/dev/kvm`, with `vm-memory`, `linux-loader`, and `virtio-queue` crates handling the rest. VirtioFS is also embedded using `vhost-user-backend` + `fuse-backend-rs`. Same approach as Firecracker -- single binary, no dependencies beyond the kernel.

**ChromeOS:** Two paths. Initially, Capsem runs inside Crostini and uses nested KVM (same embedded backend as Linux). A future optimization uses ChromeOS's `vm_concierge` D-Bus daemon to create sibling VMs, avoiding double virtualization entirely.

**Subprocess VMM (Windows):** crosvm runs as a child process with WHPX acceleration. Embedded not practical because crosvm's Windows code isn't published as standalone crates.

## Boot Sequence

```mermaid
sequenceDiagram
    participant App as capsem-app
    participant Core as capsem-core
    participant HV as Hypervisor::boot()
    participant VM as Platform VM
    participant Guest as Linux Guest

    App->>Core: start_session(config)
    Core->>HV: boot(config, vsock_ports)
    HV->>VM: Create VM (VZ/KVM fd)
    HV->>VM: Allocate guest memory
    HV->>VM: Load kernel + initrd
    HV->>VM: Generate FDT (KVM) / configure (VZ)
    HV->>VM: Attach virtio devices
    Note over VM: console, block, vsock, virtiofs
    HV->>VM: Start vCPU(s)
    VM->>Guest: Kernel boots (console=hvc0)
    Guest->>Guest: capsem-init (PID 1)
    Guest->>Guest: Mount overlayfs + VirtioFS
    Guest->>Guest: Start dnsmasq, net-proxy
    Guest->>Guest: Start pty-agent, mcp-server
    Guest-->>Core: vsock:5000 Ready
    Core-->>App: VM running, vsock wired
```

## Guest-Host Communication

All guest-host communication uses vsock (virtio socket), with four dedicated ports:

| Port | Purpose |
|------|---------|
| 5000 | Control messages (resize, heartbeat, exec) |
| 5001 | Terminal data (PTY I/O) |
| 5002 | MITM proxy (HTTPS connections) |
| 5003 | MCP gateway (tool routing) |

### Vsock Per Platform

- **macOS**: `VZVirtioSocketDevice` with ObjC delegate for connection callbacks
- **Linux / ChromeOS**: `AF_VSOCK` sockets with `vhost_vsock` kernel module
- **Windows**: crosvm's in-process virtio-vsock implementation (no kernel module needed)

The guest agent uses standard `AF_VSOCK` on all platforms -- the vsock device is transparent to guest code.

## KVM Backend Internals

```mermaid
graph TD
    subgraph Host Process
        KFD["/dev/kvm"] --> VFD["VM fd"]
        VFD --> VCPU["vCPU threads"]
        VFD --> GIC["GICv3"]
        VFD --> MEM["Guest Memory"]
    end
    subgraph MMIO Bus
        BUS["MmioBus"] --> S0["slot 0: virtio-console"]
        BUS --> S1["slot 1-2: virtio-blk"]
        BUS --> S3["slot 3: virtio-vsock"]
        BUS --> S4["slot 4+: virtio-fs"]
    end
    VCPU -- KVM_EXIT_MMIO --> BUS
    GIC -- irqfd --> S0
    GIC -- irqfd --> S4
```

### Guest Physical Address Map

| Region | Base Address | Size | Purpose |
|--------|-------------|------|---------|
| GIC Distributor | `0x0800_0000` | 64 KB | Interrupt controller |
| GIC Redistributor | `0x080A_0000` | 128 KB per vCPU | Per-CPU interrupt routing |
| Virtio MMIO | `0x0A00_0000` | 512 B per device | Device registers |
| RAM | `0x4000_0000` | Configurable | Guest memory (kernel, initrd, FDT) |

### vCPU Run Loop

Each vCPU gets a dedicated OS thread running a tight `KVM_RUN` loop. When the guest accesses a virtio MMIO register, KVM exits with `KVM_EXIT_MMIO`. The exit handler dispatches the read/write to the `MmioBus`, which routes it to the correct virtio device by address. PSCI calls (`SYSTEM_OFF`, `SYSTEM_RESET`) are handled inline to trigger VM shutdown or restart.

### FDT Generation

The KVM backend generates an aarch64 Flattened Device Tree at boot. The FDT contains: `/chosen` (kernel bootargs, initrd location), `/memory` (RAM region), `/cpus` (one node per vCPU with PSCI enable-method), a GICv3 node, an ARM generic timer node, and one `virtio_mmio` node per attached device with its MMIO base address and SPI interrupt number.

## Virtio Device Slots

| Slot | Device | IRQ (SPI) | Purpose |
|------|--------|-----------|---------|
| 0 | virtio-console | 48 | Serial console (boot logs, terminal fallback) |
| 1 | virtio-blk | 49 | Root filesystem (squashfs, read-only) |
| 2 | virtio-blk | 50 | Scratch disk (optional) |
| 3 | virtio-vsock | 51 | Guest-host vsock communication |
| 4+ | virtio-fs | 52+ | VirtioFS shared directories |

## Shared Filesystem (VirtioFS)

VirtioFS provides a POSIX-compatible shared mount between host and guest. The guest's `/root` (workspace) is a VirtioFS mount backed by a host directory.

- **macOS**: `VZVirtioFileSystemDevice` (built into Virtualization.framework)
- **Linux / ChromeOS**: Embedded VirtioFS server (`vhost-user-backend` + `fuse-backend-rs` -- same crates the standalone `virtiofsd` is built from, running in-process)
- **Windows**: crosvm `--shared-dir` with WHPX

### Embedded VirtioFS Server

On Linux, VirtioFS runs in-process as an embedded FUSE server. The FUSE protocol layer (`hypervisor/fuse/`) provides wire types, inode tracking, and file handle management. Handlers are split across three modules: `ops_meta` (INIT, LOOKUP, GETATTR, SETATTR, STATFS, FORGET), `ops_file` (OPEN, READ, WRITE, CREATE, RELEASE, FLUSH, FSYNC, LSEEK), and `ops_dir` (OPENDIR, READDIR, MKDIR, RMDIR, UNLINK, RENAME, SYMLINK, LINK).

```mermaid
sequenceDiagram
    participant GK as Guest Kernel
    participant VQ as Virtqueue
    participant W as Worker Thread
    participant FS as Host Filesystem

    GK->>VQ: FUSE request (avail ring)
    VQ->>W: channel notify
    W->>W: Gather descriptors
    W->>W: Parse FuseInHeader + dispatch
    W->>FS: Host syscall (open/read/write/stat)
    FS-->>W: Result
    W->>VQ: FUSE response (used ring)
    W->>GK: irqfd interrupt
```

See [Virtualization Security](/security/virtualization/) for threat model, path traversal analysis, and resource limits.

## Trait Design

### Hypervisor::boot()

`Hypervisor::boot()` returns `(Box<dyn VmHandle>, UnboundedReceiver<VsockConnection>)`. The channel replaces platform-specific vsock manager types. Callers receive connections as they arrive via `.recv().await` or `.try_recv()`. Each backend pushes accepted connections into the channel -- on macOS from the `VZVirtioSocketDevice` delegate callback, on Linux from `AF_VSOCK` accept loops.

### VsockConnection

`VsockConnection` holds `_lifetime_anchor: Box<dyn Send>` to keep platform resources alive for the duration of the connection. On macOS, the anchor is a `Retained<VZVirtioSocketConnection>` that prevents the Objective-C runtime from deallocating the socket object while Rust code still holds the raw fd. On Linux, the anchor is a unit type `()` because the `AF_VSOCK` file descriptor is self-contained and does not depend on an external object's lifetime. Without the anchor, the fd could become invalid when the platform object is deallocated.

### VmHandle

`VmHandle` provides lifecycle control (`stop()`, `state()`) and serial console access (`serial()`). It also supports downcast to the concrete backend type via `as_any()` for platform-specific operations. Dropping a `VmHandle` does NOT stop the VM -- callers must invoke `stop()` explicitly.

## Auto-Snapshots

The host takes rolling snapshots of the workspace directory at a configurable interval (default 5 minutes, 12 slots). Snapshots are a **host-side** operation -- the guest has no knowledge of them.

- **macOS**: APFS `clonefile()` -- instant copy-on-write
- **Linux / ChromeOS / Windows**: Hardlink-based incremental -- unchanged files are hardlinked from the previous snapshot, only changed files are copied. Near-instant for typical workloads since few files change between 5-minute intervals.

## Guest Images

Guest images are built for both `aarch64` and `x86_64`. The same kernel config, rootfs packages, init script, and agent binaries are used -- only architecture-specific compiler options differ.

| Component | arm64 | x86_64 |
|-----------|-------|--------|
| Kernel config | `defconfig.arm64` | `defconfig.x86_64` |
| Kernel output | `arch/arm64/boot/Image` | `arch/x86_64/boot/bzImage` |
| Agent target | `aarch64-unknown-linux-musl` | `x86_64-unknown-linux-musl` |

## Prerequisites

### macOS
- Apple Silicon Mac (M1+)
- macOS 13+ (Ventura)
- Xcode command-line tools (for codesigning)

### Linux
- KVM support (`/dev/kvm`)
- `vhost_vsock` kernel module
- No external VMM or virtiofsd needed -- single binary

### ChromeOS
- Crostini enabled
- Nested KVM support (newer AMD/Intel Chromebooks)

### Windows
- Windows 11
- Hyper-V Platform feature enabled
- crosvm binary (bundled with installer)
