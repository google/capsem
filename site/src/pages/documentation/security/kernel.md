---
layout: ../../../layouts/Doc.astro
title: Kernel Hardening
description: How Capsem hardens the Linux kernel running inside the VM.
lastUpdated: "2026-03-11"
tags: ["security", "kernel", "hardening"]
---

Capsem compiles its own Linux kernel from source (6.6 LTS, ~7MB vs ~30MB stock Debian). The kernel version is auto-detected from kernel.org at build time. The full config is in `images/defconfig.arm64`.

## Minimal attack surface

The kernel starts from `allnoconfig` and enables only what the VM needs. Everything else is compiled out -- not disabled at runtime, **absent from the binary**.

| Disabled subsystem | Config |
|---|---|
| Loadable modules | `MODULES=n` -- root cannot load `.ko` files |
| io_uring | `IO_URING=n` -- high-CVE-count subsystem |
| eBPF syscall | `BPF_SYSCALL=n` |
| userfaultfd | `USERFAULTFD=n` |
| 32-bit compat | `COMPAT=n` -- eliminates legacy syscall surface |
| USB, sound, DRM, wireless, Bluetooth | All `=n` |
| SCSI, ATA | `=n` -- only VirtIO block devices |
| Network filesystems | `NETWORK_FILESYSTEMS=n` |
| kexec, hibernation, SysRq | All `=n` |
| `/dev/mem`, `/dev/port` | `DEVMEM=n`, `DEVPORT=n` |
| debugfs | `DEBUG_FS=n` |
| `/proc/kallsyms` | `KALLSYMS=n` |
| IPv6 | `IPV6=n` |

## Memory hardening

| Protection | Config | Effect |
|---|---|---|
| Heap zeroing | `INIT_ON_ALLOC_DEFAULT_ON=y` | Zero-fill all heap allocations |
| Freelist randomization | `SLAB_FREELIST_RANDOMIZE=y` | Randomize SLUB freelist order |
| Freelist integrity | `SLAB_FREELIST_HARDENED=y` | Integrity checks on freelist pointers |
| Page randomization | `SHUFFLE_PAGE_ALLOCATOR=y` | Randomize page allocation order |
| Usercopy bounds | `HARDENED_USERCOPY=y` | Bounds-check `copy_to/from_user` |
| KPTI | `UNMAP_KERNEL_AT_EL0=y` | Kernel page table isolation (Meltdown mitigation) |
| Heap ASLR | `COMPAT_BRK=n` | Randomize brk heap base |

## ARM64 hardware security

Apple Silicon supports Branch Target Identification and Pointer Authentication Codes. Both are enabled:

- **BTI** (`ARM64_BTI=y`) -- hardware-enforced control flow integrity. Indirect branches must land on BTI instructions.
- **PAC** (`ARM64_PTR_AUTH=y`, `ARM64_PTR_AUTH_KERNEL=y`) -- cryptographic signatures on return addresses. Detects ROP/JOP attacks.

## Stack and code protections

- **KASLR** (`RANDOMIZE_BASE=y`) -- randomize kernel base address
- **Stack protector** (`STACKPROTECTOR_STRONG=y`) -- canaries on all functions with local arrays or address-taken variables
- **FORTIFY_SOURCE** -- compile-time and runtime bounds checking on string/memory functions
- **Strict RWX** (`STRICT_KERNEL_RWX=y`) -- no memory region is both writable and executable
- **VMAP_STACK** -- guard pages around kernel stacks to detect overflow
- **Spectre mitigation** (`HARDEN_BRANCH_PREDICTOR=y`)

## Syscall filtering

Seccomp is enabled (`SECCOMP=y`, `SECCOMP_FILTER=y`) for userspace syscall filtering as defense in depth.

## Boot cmdline hardening

The kernel command line includes runtime enforcement parameters:

```
console=hvc0 ro loglevel=1 init_on_alloc=1 slab_nomerge page_alloc.shuffle=1
```

- `init_on_alloc=1` -- runtime enforcement of heap zeroing (belt-and-suspenders with `INIT_ON_ALLOC_DEFAULT_ON`)
- `slab_nomerge` -- prevents SLUB cache merging for heap isolation
- `page_alloc.shuffle=1` -- runtime enforcement of page randomization

## Only VirtIO drivers

The VM runs inside Apple Virtualization.framework, which exposes only VirtIO devices. The kernel enables exactly those drivers and nothing else:

- `VIRTIO_PCI`, `VIRTIO_BLK`, `VIRTIO_CONSOLE` -- block and serial
- `VIRTIO_VSOCKETS` -- host-guest communication
- `HW_RANDOM_VIRTIO` -- entropy from host
- `DUMMY` -- air-gapped dummy NIC for the network proxy

No Ethernet, no real NIC drivers, no USB host controllers.

## Verification

Kernel hardening is verified at every boot by `capsem-doctor`, which checks cmdline parameters, seccomp availability, absence of `/dev/mem`, module loading disabled, and more. See [capsem-doctor](/documentation/testing/capsem-doctor) for the full test list.
