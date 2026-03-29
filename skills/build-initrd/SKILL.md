---
name: build-initrd
description: Initrd repack and guest binary management for Capsem. Use when adding new guest binaries, modifying capsem-init, changing the initrd repack process, or understanding which binaries get injected at boot vs baked into the rootfs. Covers the fast iteration loop, binary list, and how to add new guest binaries.
---

# Initrd Repack

`just run` automatically repacks the initrd before every boot. It cross-compiles guest binaries, injects them into the initrd, and `capsem-init` prefers initrd-bundled copies over rootfs copies at boot. This is the fast iteration loop (~10s) -- no full rootfs rebuild needed for guest binary changes.

## Currently repacked binaries

| Binary | What it does |
|--------|-------------|
| `capsem-init` | PID 1 init script |
| `capsem-pty-agent` | PTY-over-vsock bridge agent |
| `capsem-net-proxy` | TCP-to-vsock relay for air-gapped HTTPS proxying |
| `capsem-mcp-server` | MCP stdio-to-vsock relay for AI agent tool access |
| `capsem-doctor` | VM self-diagnostic suite (bash script) |
| `snapshots` | Snapshot management CLI (Python, FastMCP client) |
| `diagnostics/` | pytest test files for capsem-doctor |

## Adding a new guest binary

Update three places:

1. **`_pack-initrd` recipe in `justfile`** -- add the cross-compile + copy step
2. **`capsem-init` in `guest/artifacts/capsem-init`** -- add initrd-bundled fallback logic (check `/binary` before rootfs path)
3. **Binary list above** -- add it to this skill

## When to use which build path

| Changed | Command | Why |
|---------|---------|-----|
| Guest binary source (Rust agent code) | `just run` | Auto-repacks initrd with new binary |
| `capsem-init` script | `just run` | Init script is repacked into initrd |
| `guest/artifacts/diagnostics/*.py` | `just run "capsem-doctor"` | Test files repacked into initrd |
| `guest/artifacts/capsem-bashrc` | `just build-assets` | Baked into rootfs, not initrd |
| Guest config (`guest/config/`) | `just build-assets` | Affects Dockerfile rendering |
| Installed packages (apt, pip) | `just build-assets` | Baked into rootfs squashfs |

## Guest binary security

All guest binaries are deployed read-only:
- **Rootfs**: `chmod 555` in Dockerfile template (rootfs mounted read-only)
- **Initrd override**: `chmod 555` in `_pack-initrd` and `capsem-init` after copying to tmpfs
- Guest processes cannot modify these binaries at runtime

## How initrd repack works

The initrd is a gzip+cpio archive. `_pack-initrd` in the justfile:
1. Builds Rust guest binaries via `cross_compile_agent()` (on macOS: container build; on Linux: native cargo) -- outputs to `target/linux-agent/{arch}/`
2. Creates a temp directory with the binaries + init script + diagnostics
3. Sets permissions (chmod 555 for binaries, 755 for init)
4. Packs as cpio+gzip, writes to `assets/{arch}/initrd.img`

At boot, `capsem-init` checks if a binary exists in the initrd bundle (`/binary`) before falling back to the rootfs path. This means initrd copies always take priority.
