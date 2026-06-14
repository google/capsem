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
| Network (external services) | Controlled | DNS and HTTPS pass through host policy boundaries before upstream dispatch |

**What Capsem defends against:**
- Guest code escaping the VM boundary
- Guest exhausting host CPU, memory, disk, or file descriptors
- Guest accessing network services blocked by profile or corporate rules
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
| **Network isolation** | Air-gapped NIC, DNS proxy, iptables, MITM proxy | DNS and HTTPS are funneled through audited host policy handlers |
| **Filesystem sandboxing** | VirtioFS with path validation, resource limits | Guest confined to workspace directory |
| **Build verification** | Code signing, notarization, SBOM, OBOM | Host binary and VM base-image integrity |

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

**Network boundary (DNS + network intercept):** Guest DNS and HTTPS traffic are
redirected to guest proxy binaries and forwarded over vsock to host handlers.
HTTPS is terminated at the host, normalized into `SecurityEvent` fields,
evaluated by the shared rule rail, and forwarded to real upstream only after
enforcement allows it. Runtime materialization and ledger materialization are
separate: upstream may need real protocol bytes, while session DB, structured
logs, routes, and UI stats receive only the ledger-safe projection produced by
logging plugins. Per-session telemetry records every request and DNS query.

**Filesystem boundary (VirtioFS):** The host VirtioFS server validates all path components, canonicalizes symlinks, and rejects any path that resolves outside the shared workspace. Resource limits prevent guest-driven host exhaustion.

## Per-Layer Documentation

- [Kernel Hardening](/security/kernel-hardening/) -- guest kernel lockdown configuration
- [Network Isolation](/security/network-isolation/) -- air-gapped networking and MITM proxy
- [Virtualization Security](/security/virtualization/) -- VirtioFS sandboxing and hypervisor hardening
- [Build Verification](/security/build-verification/) -- code signing, notarization, and supply chain
