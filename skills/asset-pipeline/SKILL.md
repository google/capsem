---
name: asset-pipeline
description: Asset building, manifest format, hash verification, and boot-time resolution for Capsem VM images. Use when debugging boot failures, manifest issues, hash mismatches, or understanding how assets flow from build to boot.
---

# Asset Pipeline

How VM assets (kernel, initrd, rootfs) are built, checksummed, resolved, and verified at boot.

## Versioning

Binary and asset versions are **independent**:
- **Binary**: `1.0.{unix_timestamp}` -- changes every build
- **Assets**: `YYYY.MMDD.patch` -- changes only on kernel/rootfs/initrd rebuilds

The manifest tracks both with compatibility ranges (`min_binary`, `min_assets`).

## Key Commands

| Command | When to use |
|---------|-------------|
| `just build-assets` | Full rebuild: kernel + rootfs + checksums (slow, needs docker) |
| `just shell` | Daily driver: repack initrd, build, sign, boot (~10s) |
| `just shell "capsem-doctor"` | Verify VM boots correctly after changes |

## File Locations

| What | Where |
|------|-------|
| Guest config (TOML) | `guest/config/` |
| Guest artifacts | `guest/artifacts/` |
| Built assets (dev) | `assets/{arch}/vmlinuz, initrd.img, rootfs.squashfs` |
| Installed assets | `~/.capsem/assets/{name}-{hash16}.{ext}` (flat, hash-based) |
| Manifest | `assets/manifest.json` |
| Checksums | `assets/B3SUMS` |
| Manifest regenerator | `scripts/gen_manifest.py` |
| Asset types + cleanup | `crates/capsem-core/src/asset_manager.rs` |
| Hash extraction for build.rs | `crates/capsem-core/src/manifest_compat.rs` |

## Manifest Format (v2)

```json
{
  "format": 2,
  "assets": {
    "current": "2026.0415.1",
    "releases": {
      "2026.0415.1": {
        "date": "2026-04-15",
        "deprecated": false,
        "min_binary": "1.0.0",
        "arches": {
          "arm64": {
            "vmlinuz": { "hash": "<64-char blake3>", "size": 7797248 },
            "initrd.img": { "hash": "...", "size": 2270154 },
            "rootfs.squashfs": { "hash": "...", "size": 454230016 }
          }
        }
      }
    }
  },
  "binaries": {
    "current": "1.0.1776269479",
    "releases": {
      "1.0.1776269479": {
        "date": "2026-04-15",
        "deprecated": false,
        "min_assets": "2026.0415.1"
      }
    }
  }
}
```

Two producers: `docker.py:generate_checksums()` (full build) and `scripts/gen_manifest.py` (initrd repack). Both produce v2 format.

## Disk Layouts

**Dev** (repo `assets/` dir -- logical names, per-arch subdirs):
```
assets/arm64/vmlinuz
assets/arm64/initrd.img
assets/arm64/rootfs.squashfs
assets/manifest.json
```

**Installed** (`~/.capsem/assets/` -- flat, hash-based filenames):
```
manifest.json
vmlinuz-2c0bd752db929642
initrd-e5e910e9ab38b873.img
rootfs-89eb92b83534d9d0.squashfs
```

Hash-based naming: `{stem}-{hash[..16]}{ext}`. Same hash = same file across versions = natural dedup.

## Boot-Time Resolution

1. **Dev mode**: Service detects arch subdirs, passes `--kernel assets/{arch}/vmlinuz` etc. to capsem-process
2. **Installed mode**: Service reads v2 manifest, resolves `ManifestV2::resolve(binary_version, arch, base_dir)` to get hash-based file paths, passes `--kernel`, `--initrd`, `--rootfs` individually to capsem-process
3. **Hash check at boot**: `VmConfig::builder().build()` verifies BLAKE3 against compile-time hashes if available

## Cleanup

`cleanup_unused_assets(base_dir, manifest)` removes hash-named files not referenced by any non-deprecated asset release. Also removes legacy `v*/` directories.

## Common Issues

**Hash mismatch at boot**: Assets on disk don't match the hashes baked into the binary. Fix: `just shell` (repacks initrd, regenerates manifest, touches build.rs to force recompile).

**Hashes silently skipped**: If `build.rs` can't extract hashes (manifest missing, wrong format), `option_env!()` returns `None` and verification is skipped.
