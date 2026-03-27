---
name: site-architecture
description: Capsem system architecture -- how the host, guest VM, network proxy, and terminal I/O work together. Use when you need to understand the system design to write code, review changes, write documentation, or debug cross-component issues. Covers the host/guest split, vsock ports, storage modes, network policy, MITM proxy, boot sequence, and key source files.
---

# Capsem Architecture

## System overview

Capsem sandboxes AI agents in air-gapped Linux VMs on macOS using Apple's Virtualization.framework (with a KVM backend for Linux). The system has four layers:

- **Host app** (Tauri + Rust): creates the VM, manages vsock connections, runs the MITM proxy, serves the frontend
- **Guest init** (`capsem-init`): PID 1, sets up air-gapped networking, launches daemons
- **Guest agent** (`capsem-pty-agent`): bridges PTY I/O over vsock to the host
- **Guest net proxy** (`capsem-net-proxy`): redirects HTTPS traffic to host MITM proxy via vsock

## Host-guest communication

```
Frontend (xterm.js) <-> Tauri commands <-> vsock <-> Guest PTY agent <-> bash
```

Terminal I/O: xterm.js `onData` -> Tauri `serial_input` -> vsock port 5001 -> guest PTY. Reverse: guest PTY -> vsock -> CoalesceBuffer (8ms/64KB) -> Tauri event -> xterm.js `write`.

Serial console stays active for kernel boot logs. Terminal I/O switches to vsock once the guest agent sends `Ready`.

### Vsock ports

| Port | Purpose |
|------|---------|
| 5000 | Control messages (resize, heartbeat, exec) |
| 5001 | Terminal data (PTY I/O) |
| 5002 | MITM proxy (HTTPS connections) |
| 5003 | MCP gateway (tool routing, NDJSON passthrough) |

## Storage modes

Selected by kernel cmdline `capsem.storage=virtiofs` (default) or absence (block mode).

**VirtioFS mode** (default):
```
~/.capsem/sessions/{id}/
  system/rootfs.img    # ext4 loopback (2GB sparse) -- overlayfs upper
  workspace/           # VirtioFS files for /root (host-visible)
  auto_snapshots/      # Rolling ring buffer (12 APFS clones, 5min interval)
```

Boot sequence: squashfs -> VirtioFS mount -> loopback ext4 -> overlayfs -> bind-mount workspace.

Why ext4 loopback: Apple VZ's VirtioFS doesn't support `mknod` (whiteout creation), so overlayfs can't use VirtioFS directly as upper.

**Block mode** (legacy): tmpfs overlay + scratch disk. No host file visibility, no snapshots.

## Network architecture

The guest is air-gapped. No real NIC, no real DNS, no direct internet access.

1. `capsem-init` creates a dummy0 NIC with fake DNS (dnsmasq)
2. iptables redirects all port 443 traffic to `capsem-net-proxy` on localhost:10443
3. `capsem-net-proxy` bridges each TCP connection to host vsock port 5002
4. Host MITM proxy terminates TLS using per-domain minted certs (signed by static Capsem CA)
5. Host inspects HTTP request, applies domain + HTTP policy, forwards to real upstream
6. Full telemetry recorded to session DB (domain, method, path, status, headers, body preview)

### Network policy

- User config: `~/.capsem/user.toml` -- domain allow/block lists + HTTP rules
- Corp config: `/etc/capsem/corp.toml` -- enterprise lockdown (MDM-distributed)
- Merge: corp overrides user entirely per field; unspecified fields fall through
- HTTP rules: `[[network.rules]]` with method+path matching per domain

### MITM CA

- Static CA: `config/capsem-ca.key` + `config/capsem-ca.crt` (ECDSA P-256)
- Baked into rootfs via `update-ca-certificates` + certifi patch
- Guest trusts it via system store + env vars (`REQUESTS_CA_BUNDLE`, `NODE_EXTRA_CA_CERTS`, `SSL_CERT_FILE`)

## Ephemeral VM model (invariants)

**VirtioFS mode**: fresh workspace + sparse rootfs.img per session. Host creates empty dirs, guest formats on first boot.

**Block mode**: `mke2fs` runs unconditionally at boot. Overlay upper is always tmpfs.

Never make the overlay upper layer persistent. To add packages: edit guest config and `just build-assets`.

## Key source files

Read `references/key-files.md` for the full annotated source map.

## Tauri v2 reference

Read `references/tauri-v2.md` for Tauri IPC patterns, commands, capabilities, and permissions.

## Crate architecture

- **`capsem-core`**: all shared logic (VM, network, policy, telemetry, config). This is where business logic lives.
- **`capsem-app`**: thin Tauri shell. Wires IPC commands to core. No business logic.
- **`capsem-agent`**: thin guest binary. PTY bridge + net proxy + MCP server. Cross-compiled for aarch64-linux-musl.
- **`capsem-logger`**: session DB schema and queries.
- **`capsem-proto`**: shared protocol types (control messages, MCP frames).
