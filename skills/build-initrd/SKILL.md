---
name: build-initrd
description: Initrd repack and guest binary management. Use when adding or changing a guest binary, modifying capsem-init, or iterating on the initrd without a full rebuild.
---

# Initrd Repack

`just exec` composes `capsem-gate pack-initrd` before boot. The gate stages the
configured guest binaries, atomically repacks the host initrd, regenerates its
manifest and hash aliases, and then materializes runtime config. `capsem-init`
prefers these initrd copies over the rootfs copies.

The authoritative inventory and paths are `[initrd]` in `config/gate.toml`.
Do not add a second list to a recipe, test, or gate module.

## Currently repacked binaries

| Binary | What it does |
|--------|-------------|
| `capsem-init` | PID 1 init script |
| `capsem-pty-agent` | PTY-over-vsock bridge agent |
| `capsem-net-proxy` | TCP-to-vsock relay for air-gapped HTTPS proxying |
| `capsem-dns-proxy` | UDP/TCP DNS-to-vsock relay |
| `capsem-mcp-server` | MCP stdio-to-vsock relay for AI agent tool access |
| `capsem-sysutil` | Guest suspend helper via vsock:5004; in-VM shutdown commands are disabled |
| `capsem-bench-rs` | Rust guest benchmark helper |
| `capsem-doctor` | VM self-diagnostic suite (bash script) |
| `capsem-bench` | Guest benchmark driver (bash script) |
| `snapshots` | Snapshot management CLI (Python, FastMCP client) |
| `capsem_bench/` | Python benchmark support tree |
| `diagnostics/` | pytest test files for capsem-doctor |

## Adding a new guest binary

Update two authorities, then their behavior tests:

1. **`[initrd].binaries` in `config/gate.toml`** -- the repacker, static gate,
   build-chain test, and release asset rail all consume it.
2. **`guest/artifacts/capsem-init`** -- add the initrd-first deployment/fallback
   behavior if the new process is launched during boot.

Update this table and add the relevant unit, archive, and in-VM behavior tests.

## When to use which build path

| Changed | Command | Why |
|---------|---------|-----|
| Guest binary source (Rust agent code) | `just exec` | Auto-repacks initrd with new binary |
| `capsem-init` script | `just exec` | Init script is repacked into initrd |
| `guest/artifacts/diagnostics/*.py` | `just exec "capsem-doctor"` | Test files repacked into initrd |
| `guest/artifacts/capsem-bashrc` | `just _build-assets <profile>` | Baked into rootfs, not initrd |
| Profile package/root/build inputs (`config/profiles/<id>/`) | `just _build-assets <profile>` | Affects profile-derived rootfs rendering |
| Installed packages (apt, pip) | `just _build-assets <profile>` | Baked into the profile rootfs asset |

## Guest binary security

All guest binaries are deployed read-only:
- **Rootfs**: `chmod 555` in Dockerfile template (rootfs mounted read-only)
- **Initrd override**: `[initrd].binary_mode` in the gate repacker and `capsem-init` after copying to tmpfs
- Guest processes cannot modify these binaries at runtime

## How initrd repack works

The initrd is a gzip+cpio archive. `src/capsem/gate/initrd.py`:

1. Resolves the exact content-addressed Rust payload generation under the
   configured `cache/target/build/linux-agent/<arch>/` tree. Source bytes,
   modes, paths, toolchain inputs, and the sealed builder image define the
   generation; checkout timestamps never do.
2. Unpacks the exact target into a temporary directory.
3. Replaces the config-owned binaries, files, trees, and init script at their
   configured modes.
4. Packs beside the target and uses `AtomicReplace`, preserving old hardlinks.
5. Regenerates the manifest and hash aliases whenever it mutates public assets.

The complete IronBank asset graph has a visible `assets.pack-initrds` frontier
after both architecture lanes and before merge/boot. It repacks every
profile/architecture private target. The CI-facing `build-assets rootfs|all`
rail uses the same primitive before upload; `kernel` intentionally keeps its
minimal initrd until the following rootfs step.

At boot, `capsem-init` checks if a binary exists in the initrd bundle (`/binary`) before falling back to the rootfs path. This means initrd copies always take priority.

## Lesson: permissions are set in TWO places

Guest binary permissions must be 555 (read+execute, no write). There are two independent places that set permissions and both must agree:

1. **Dockerfile.rootfs.j2** -- `chmod 555` when copying into the profile rootfs asset
2. **`[initrd].binary_mode` through the gate repacker** -- mode on the initrd copy

The initrd copy wins at runtime because it overlays the rootfs. Keep both
locations at 555 and prove the final archive with
`tests/capsem-build-chain/test_pack_initrd.py`.
