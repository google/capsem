---
name: asset-pipeline
description: Asset building, manifest format, hash verification, and boot-time resolution for Capsem VM images. Use when debugging boot failures, manifest issues, hash mismatches, or understanding how assets flow from build to boot.
---

# Asset Pipeline

How VM assets (kernel, initrd, rootfs) are built, checksummed, resolved, and verified at boot.

## Key Commands

| Command | When to use |
|---------|-------------|
| `just build-assets` | Full rebuild: kernel + rootfs + checksums (slow, needs docker) |
| `just run` | Daily driver: repack initrd, build, sign, boot (~10s) |
| `just run "capsem-doctor"` | Verify VM boots correctly after changes |
| `capsem-builder build guest/ --arch arm64 --template rootfs` | Build one template for one arch |

## File Locations

| What | Where |
|------|-------|
| Guest config (TOML) | `guest/config/` |
| Guest artifacts | `guest/artifacts/` |
| Jinja templates | `src/capsem/builder/templates/` |
| Built assets | `assets/{arch}/vmlinuz, initrd.img, rootfs.squashfs` |
| Manifest | `assets/manifest.json` |
| Checksums | `assets/B3SUMS` |
| Builder CLI | `src/capsem/builder/cli.py` |
| Builder docker logic | `src/capsem/builder/docker.py` |
| Manifest regenerator | `scripts/gen_manifest.py` |
| Compile-time hash extraction | `crates/capsem-app/build.rs` |
| Runtime asset resolution | `crates/capsem-app/src/assets.rs` |
| Boot config + hash verification | `crates/capsem-core/src/vm/config.rs` |
| Asset download manager | `crates/capsem-core/src/asset_manager.rs` |
| Shared hash extraction logic | `crates/capsem-core/src/manifest_compat.rs` |

## Manifest Format

Per-arch nested format (filenames are **bare**, not arch-prefixed):

```json
{
  "releases": {
    "0.12.1": {
      "arm64": {
        "assets": [
          {"filename": "vmlinuz", "hash": "<64-char blake3>", "size": 7797248}
        ]
      }
    }
  }
}
```

Two producers: `docker.py:generate_checksums()` (full build) and `scripts/gen_manifest.py` (initrd repack). Both detect arch subdirs and produce per-arch format.

## Boot-Time Resolution

1. **Find assets dir**: env var -> .app bundle -> `./assets` -> `../../assets`. Checks `{dir}/{arch}/vmlinuz` first, then flat.
2. **Find rootfs**: bundled -> `~/.capsem/assets/v{version}/` -> legacy flat
3. **Download if missing**: manifest-driven download with BLAKE3 verification and HTTP resume
4. **Hash check at boot**: `VmConfig::builder().build()` verifies BLAKE3 of kernel, initrd, rootfs against compile-time hashes. Mismatch prevents boot.

## Common Issues

**Hash mismatch at boot**: Assets on disk don't match the hashes baked into the binary. Happens when assets are rebuilt without recompiling the app. Fix: `just run` (repacks initrd, regenerates manifest, touches build.rs to force recompile).

**Manifest not found**: `create_asset_manager()` checks both the resolved assets dir and its parent. Per-arch layout puts `manifest.json` at `assets/manifest.json` while resolved dir is `assets/arm64/`.

**Hashes silently skipped**: If `build.rs` can't extract hashes (manifest missing, wrong version, wrong format), `option_env!()` returns `None` and verification is skipped. The `manifest_compat` tests guard against this.

**Wrong arch assets**: Impossible at runtime. `host_arch()` is compile-time, and `build.rs` extracts hashes for the target arch only. A binary built for arm64 will only look for arm64 assets.
