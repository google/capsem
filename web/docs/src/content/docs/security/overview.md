---
title: Security Model
description: Capsem's threat model, defense layers, and trust boundaries.
sidebar:
  order: 10
---

Capsem sandboxes AI agents inside Linux VMs and splits sensitive host work into
confined processes. The guest is fully untrusted. The trusted computing base is
the coordinator service, host kernel, hypervisor, and verified package/runtime
artifacts; a parser worker running as the desktop user does not automatically
receive the coordinator's authority.

## Threat Model

| Party | Trust Level | Goal |
|-------|------------|------|
| Host kernel, hypervisor, and `capsem-service` coordinator | Trusted | Create resources, grant bounded authority, contain the guest and workers |
| Gateway, VM owner, proxy, ledger, router, and MCP workers | Confined | Perform one role with explicit paths or connected descriptors |
| Guest (AI agent, user code, guest kernel) | Untrusted | May attempt sandbox escape, resource exhaustion, data exfiltration |
| Network (external services) | Controlled | DNS and HTTPS pass through host policy boundaries before upstream dispatch |

**What Capsem defends against:**
- Guest code escaping the VM boundary
- Guest exhausting host CPU, memory, disk, or file descriptors
- Guest accessing network services blocked by user or corporate rules
- Unaudited data exfiltration via HTTPS

**What Capsem does not defend against:**
- A compromised coordinator service, host kernel, or hypervisor
- Unrelated same-user host applications outside Capsem's sandbox and lifecycle
- Hardware side-channel attacks (mitigated by OS/firmware, not Capsem)
- Denial of service against the guest itself (the guest is disposable)

## Defense Layers

| Layer | Mechanism | What It Protects |
|-------|-----------|-----------------|
| **Hardware virtualization** | Apple VZ / KVM | Guest cannot access host memory, devices, or kernel |
| **Kernel hardening** | No modules, no debugfs, no IPv6, no swap, read-only rootfs | Reduces guest kernel attack surface |
| **Network isolation** | Air-gapped NIC, DNS proxy, iptables, MITM proxy | DNS and HTTPS are funneled through audited host policy handlers |
| **Filesystem sandboxing** | VirtioFS with path validation, resource limits | Guest confined to workspace directory |
| **Host worker confinement** | Seatbelt or Landlock + seccomp, descriptor grants, fresh generations | Parser/storage/relay compromise is limited to the worker's current capabilities |
| **Build verification** | Code signing, notarization, SBOM, OBOM | Host binary and VM base-image integrity |

## Trust Boundaries

```
+------------------+          +-----------------------+
|   Guest VM       |  virtio  |   Host (Capsem)       |
|                  |<-------->|                       |
|  AI agent        |  vsock   |  Terminal bridge      |
|  Guest kernel    |  virtio  |  MITM proxy           |
|  Guest userland  |  fs      |  VirtioFS server      |
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
logs, routes, and UI stats receive only the ledger-safe event output produced by
logging plugins. Per-session telemetry records every request and DNS query.

**Filesystem boundary (VirtioFS):** The host VirtioFS server validates every
component and performs guest-writable filesystem operations relative to an
already-open workspace descriptor with no-follow semantics. Symlinks cannot
redirect a later host open outside the share. Resource limits prevent
guest-driven host exhaustion.

**Host-process boundary:** The service opens resources and grants connected
descriptors to generation-bound workers. The proxy has no filesystem or socket
creation authority; the ledger can access one session directory and no
network; the gateway and VM owner receive exact path and inherited-listener
grants. See [Host Process Isolation](/architecture/host-isolation/).

## Per-Layer Documentation

- [Kernel Hardening](/security/kernel-hardening/) -- guest kernel lockdown configuration
- [Network Isolation](/security/network-isolation/) -- air-gapped networking and MITM proxy
- [Virtualization Security](/security/virtualization/) -- VirtioFS sandboxing and hypervisor hardening
- [Build Verification](/security/build-verification/) -- code signing, notarization, and supply chain
