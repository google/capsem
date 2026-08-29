---
name: asset-pipeline
description: Asset manifests, hash verification, and boot-time resolution for Capsem VM images. Use when debugging boot failures, manifest issues, or hash mismatches.
---

# Asset Pipeline

How VM assets (kernel, initrd, rootfs) are built, checksummed, resolved, and verified at boot.

## Reference routing

- Read `references/manifest-and-storage.md` before changing the v2 manifest
  schema, digest generation or enrichment, corporate/local manifest inputs,
  installed asset layout, package manifest metadata, or cache naming.
- Read `references/release-channel-publication.md` before changing channel
  graph generation, binary/profile publication, channel switching, deployment,
  Cloudflare readiness, evidence or attestation checks, or cache headers.

## Manifest Authority

The selected manifest is the bible: if an artifact is not in that manifest, it
does not exist for the release, update, cache, or boot path. Never infer
membership from a cache directory, release attachment, filename, channel name,
or prior run. Fetch mutable manifests fresh. Cache only immutable artifact
bytes, address each cache entry directly by the digest recorded in the
manifest, and re-verify that digest before every use. Artifact cache identity
is channel-independent; the manifest decides which digest set belongs to a
channel/profile at that moment.

## Versioning

Binary and asset versions are **independent**:
- **Binary**: `1.3.{unix_timestamp}` on the current release line -- changes every build
- **Assets**: `YYYY.MMDD.patch` -- changes only on kernel/rootfs/initrd rebuilds

The manifest tracks both with compatibility ranges (`min_binary`, `min_assets`).
Runtime asset selection enforces both directions: older binaries do not hydrate
asset releases whose `min_binary` requires a newer binary, and new
session/download selection skips releases marked `deprecated: true`.

## Key Commands

| Command | When to use |
|---------|-------------|
| `just _build-assets` | Full rebuild: kernel + rootfs + checksums (slow, needs docker) |
| `just shell` | Daily driver: repack initrd, build, sign, boot (~10s) |
| `just shell "capsem-doctor"` | Verify VM boots correctly after changes |

On macOS, `just _build-assets`, `just _pack-initrd`, and any Docker-backed
asset recipe depend on Colima. If Docker cannot connect but Colima appears to
be running, follow `/dev-setup`'s Colima recovery discipline before treating
the asset build as blocked: check `colima list`, `docker version`, and
`colima ssh -- docker ps`; then try `colima stop && colima start` once and
rerun the failing recipe.

## File Locations

| What | Where |
|------|-------|
| Profile source config | `config/profiles/<id>/` |
| Guest artifacts | `guest/artifacts/` |
| Built assets (dev) | `target/assets/{arch}/vmlinuz, initrd.img, rootfs.erofs` |
| Installed assets | `~/.capsem/assets/{name}-{hash16}.{ext}` (flat, hash-based) |
| Manifest | `target/assets/manifest.json` |
| Asset channel deploy root | `target/distribution/` |
| Asset channel manifest | `target/distribution/assets/<channel>/manifest.json` |
| Asset channel human site | `build_system/release_site/` Astro app, built from `target/distribution/` JSON |
| Checksums | `target/assets/B3SUMS` |
| Manifest generator | `capsem-admin manifest generate <assets_dir>` |
| Asset types + cleanup | `crates/capsem-core/src/asset_manager.rs` |
| Hash extraction for build.rs | `crates/capsem-core/src/manifest_compat.rs` |

## Boot-Time Resolution

1. **Dev mode**: Service detects arch subdirs, passes `--kernel target/assets/{arch}/vmlinuz` etc. to capsem-process
2. **Installed mode**: Service reads v2 manifest, resolves `ManifestV2::resolve(binary_version, arch, base_dir)` to get hash-based file paths, passes `--kernel`, `--initrd`, `--rootfs` individually to capsem-process
3. **Hash check at boot**: `VmConfig::builder().build()` verifies BLAKE3 against compile-time hashes if available

## Cleanup

`cleanup_unused_assets(base_dir, manifest)` removes hash-named files not referenced by any non-deprecated asset release. Also removes legacy `v*/` directories.
Existing VM pins are preserved by the VM pinning rail; deprecation blocks new
selection rather than rewriting running VMs.

Persistent resume has its own immutable authority: the registry's profile and
asset pins plus the session's saved validated `vm/active_profile.toml`. Never
materialize today's profile over that file before deciding whether the old VM
may boot. A normal profile/image advance and a deprecated pin preserve the VM;
an explicit installed-manifest revocation, a corrupt saved profile, a missing
pinned asset, or invalid rootfs geometry blocks it. Any cached resume verdict
must fingerprint the saved profile, installed manifest, rootfs metadata, and
pinned asset metadata so a revocation or repair is visible immediately.

## Common Issues

**Hash mismatch at boot**: Assets on disk don't match the hashes baked into the binary. Fix: `just shell` (repacks initrd, regenerates manifest, touches build.rs to force recompile).

**Hash mismatch where expected and actual look identical**: the two values differ
only by an algorithm tag. Digests reach boot in two spellings — asset manifests
carry bare hex, release-graph digests and the profile pins derived from them
carry `blake3:<hex>`.

`VmConfigBuilder::verify_hash` resolves both, in the one place that decides what
an expected hash means, and refuses a non-blake3 algorithm outright rather than
letting a `sha256:` pin masquerade as corruption it can never match. Do not add
a second reconciliation at a call site.

**Log pins in full, never truncated.** A 16-character slice renders both
spellings as plausible prefixes (`blake3:de1d58193` looks like a hash), so a
truncated audit line hides exactly the mismatch it exists to catch.

**Boot verifies the *booting profile's* pins.** A channel carries one image set
per profile, so no channel-wide pointer can answer which hashes apply — the
caller passes `expected_asset_hashes` for the profile it is starting. Absent is
a hard error, not permission to boot unverified.

**Hashes silently skipped**: If `build.rs` can't extract hashes (manifest missing, wrong format), `option_env!()` returns `None` and verification is skipped.
