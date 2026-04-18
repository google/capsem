---
title: Security Model
description: Capsem's threat model, defense layers, and trust boundaries.
sidebar:
  order: 10
---

Capsem sandboxes AI agents inside Linux VMs. The security model treats the guest as fully untrusted and the host as the trusted computing base.

## Threat Model

| Party | Trust Level | Goal |
|-------|------------|------|
| Host (Capsem binary, macOS/Linux kernel) | Trusted | Contain guest escape, protect host resources |
| Guest (AI agent, user code, guest kernel) | Untrusted | May attempt sandbox escape, resource exhaustion, data exfiltration |
| Network (external services) | Controlled | All traffic audited via MITM proxy; allow/deny per domain+HTTP path |

**What Capsem defends against:**
- Guest code escaping the VM boundary
- Guest exhausting host CPU, memory, disk, or file descriptors
- Guest accessing network services outside the allow list
- Unaudited data exfiltration via HTTPS

**What Capsem does not defend against:**
- Compromised host processes (they already have equivalent privileges)
- Hardware side-channel attacks (mitigated by OS/firmware, not Capsem)
- Denial of service against the guest itself (the guest is disposable)

## Defense Layers

| Layer | Mechanism | What It Protects |
|-------|-----------|-----------------|
| **Hardware virtualization** | Apple VZ / KVM | Guest cannot access host memory, devices, or kernel |
| **Kernel hardening** | No modules, no debugfs, no IPv6, no swap, read-only rootfs | Reduces guest kernel attack surface |
| **Network isolation** | Air-gapped NIC, fake DNS, iptables, MITM proxy | All traffic funneled through audited proxy |
| **Filesystem sandboxing** | VirtioFS with path validation, resource limits | Guest confined to workspace directory |
| **Build verification** | Code signing, notarization, SBOM | Host binary integrity |

## Trust Boundaries

```
+------------------+          +-----------------------+
|   Guest VM       |  virtio  |   Host (Capsem)       |
|                  |<-------->|                       |
|  AI agent        |  vsock   |  Terminal bridge      |
|  Guest kernel    |  virtio  |  MITM proxy           |
|  Guest userland  |  fs      |  VirtioFS server      |
|                  |          |  Snapshot scheduler    |
+------------------+          +-----------------------+
                                        |
                                   Host kernel
                                   (macOS / Linux)
```

**Guest/host boundary (virtio):** All communication uses virtio devices (console, vsock, VirtioFS). The guest cannot directly access host memory or syscalls. The hypervisor validates all virtio descriptor chains.

**Network boundary (MITM proxy):** Guest HTTPS traffic is terminated at the host, inspected against domain + HTTP policy, and forwarded to real upstream. Per-session telemetry records every request.

**Filesystem boundary (VirtioFS):** The host VirtioFS server validates all path components, canonicalizes symlinks, and rejects any path that resolves outside the shared workspace. Resource limits prevent guest-driven host exhaustion.

## Per-Layer Documentation

- [Kernel Hardening](/security/kernel-hardening/) -- guest kernel lockdown configuration
- [Network Isolation](/security/network-isolation/) -- air-gapped networking and MITM proxy
- [Virtualization Security](/security/virtualization/) -- VirtioFS sandboxing and hypervisor hardening
- [Build Verification](/security/build-verification/) -- code signing, notarization, and supply chain
