---
name: asset-pipeline
description: Asset building, manifest format, hash verification, and boot-time resolution for Capsem VM images. Use when debugging boot failures, manifest issues, hash mismatches, or understanding how assets flow from build to boot.
---

# Asset Pipeline

How VM assets (kernel, initrd, rootfs) are built, checksummed, resolved, and verified at boot.

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
| `just build-assets` | Full rebuild: kernel + rootfs + checksums (slow, needs docker) |
| `just shell` | Daily driver: repack initrd, build, sign, boot (~10s) |
| `just shell "capsem-doctor"` | Verify VM boots correctly after changes |

On macOS, `just build-assets`, `just _pack-initrd`, and any Docker-backed
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
| Built assets (dev) | `assets/{arch}/vmlinuz, initrd.img, rootfs.erofs` |
| Installed assets | `~/.capsem/assets/{name}-{hash16}.{ext}` (flat, hash-based) |
| Manifest | `assets/manifest.json` |
| Asset channel deploy root | `target/release-channel/` |
| Asset channel manifest | `target/release-channel/assets/<channel>/manifest.json` |
| Checksums | `assets/B3SUMS` |
| Manifest generator | `capsem-admin manifest generate <assets_dir>` |
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
            "rootfs.erofs": { "hash": "...", "size": 454230016 }
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

The public producer is `capsem-admin manifest generate <assets_dir>`. Full
asset builds and initrd repacks feed that same profile-derived build rail so local, CI, and
corporate manifests use one contract. Corporate VM asset channels use
`capsem update --assets --manifest <URL>`; `--manifest` is URL-shaped, so local
custom manifests use `file:///absolute/path/to/manifest.json`, while hosted corp
channels use `https://...` or `http://...`. Do not use `capsem update --corp`
for asset channels: `--corp` provisions corporate policy config, while
corporate VM asset channels stay on the shared manifest/update path.

The public asset channel is generated from that manifest with
`capsem-admin assets channel build`. Do not invent a separate release-channel
source tree or alternate manifest format. The generated deploy root is
`target/release-channel/`; the machine artifact is
`assets/<channel>/manifest.json` under that root, so the stable public URL is
`https://release.capsem.org/assets/stable/manifest.json`.
Immutable VM blobs for that manifest live under
`assets/releases/<asset-version>/<arch>-<logical_name>` in the same deploy root.
For example, a stable manifest whose current asset release is `2026.0627.1`
hydrates `arm64-vmlinuz` from
`https://release.capsem.org/assets/releases/2026.0627.1/arm64-vmlinuz`.
The generated `health.json` is the compact machine-readable release-site index:
schema `capsem.assets_channel.health.v1`, active manifest URL, immutable asset
base URL, current binary/assets versions, current asset file URLs, VM OBOM
references, host SBOM references, binary file metadata when present, an explicit `updates` block with
`latest` targets for binary/assets/profile/image freshness checks, and a
profile catalog block with revision, published catalog artifact path, BLAKE3 digest,
compatibility minimums, and whether the advertised profile catalog requires a
newer binary or VM asset set, plus host and VM asset attestation references
with predicate type and `gh attestation verify` command hints.
Host SBOM evidence is incomplete unless `github_attestations_host_sbom` is
present, points at the published `capsem-sbom.spdx.json` evidence, and covers
every published host package subject.
It also carries dated asset release history, including deprecated VM asset releases;
deprecated releases remain auditable but are not candidates for new
session/download selection.

The manual asset workflow is `.github/workflows/release-assets.yaml`. It should
remain explicit/manual, build VM assets, publish changed blobs to an immutable
`assets-v<asset-version>` GitHub Release, attest the arch-prefixed `vmlinuz`,
`initrd.img`, `rootfs.erofs`, and `obom.cdx.json` subjects, upload
`target/release-channel/` as the `asset-channel-preview` artifact, and call
`.github/workflows/release-channel.yaml` to deploy `release.capsem.org` only
after the asset manifest, blobs, and channel checks have been generated.
Before the asset delta check and channel build, the workflow preserves the live
channel's `binaries` metadata in the generated asset manifest so VM asset
releases do not erase package hashes, host SBOM evidence, or binary attestation
state from `release.capsem.org`. Manual VM asset releases do not accept or
publish a binary-version override; binary release metadata is owned by the
tag-triggered binary rail.
The delta emits both `asset_changed` and `asset_blobs_changed`: metadata-only
asset release changes, such as deprecating an older VM asset release, still
deploy the release channel without republishing immutable VM blobs. The
`asset-release-plan`, GitHub Release upload, and provenance attestation steps
run only when `asset_blobs_changed` is true. The first channel bootstrap may
have no host binary evidence yet because the tag-triggered binary rail has not
recorded package files, the canonical `capsem-sbom.spdx.json` host SBOM
reference, or host binary attestations; once binary files are published,
missing host SBOM evidence is release-blocking.
Later publications still compare
against the live previous manifest and skip deployment only when current VM blob hashes, asset release metadata, and manifest policy are all unchanged. Manifest policy includes channel-visible fields such as `refresh_policy`.
`build-ledger.log` and `B3SUMS` are debug evidence unless deliberately promoted
to separate published evidence.
The deploy smoke rejects stale public HTML: the fetched index page must show the
same current binary, current VM asset version, asset release date, generated
timestamp, profile revision, profile catalog URL, profile update source, and
channel manifest path as the fetched health JSON and manifest.
The deploy smoke also rejects stale `health.json` summary state: `ok`, channel,
published state, index/health URLs, and top-level binary/assets versions must
match the active channel manifest and release-site layout.
The deploy smoke also verifies every manifest asset release row in
`health.json`, including date, deprecated state, deprecation date, and minimum
binary compatibility, so metadata-only deprecation changes cannot leave the
public release history stale.
The deploy smoke also verifies that the binary update target, state, source, and
package file metadata match the canonical binary metadata.
The deploy smoke also verifies that the VM asset update target, manifest, base
URL, compatibility, and newer-version requirements match the canonical asset
metadata.
The deploy smoke also verifies that the profile update hash, compatibility, and
newer-version requirements match the canonical profile catalog metadata.
The deploy smoke also verifies that image freshness remains explicitly
unpublished until image release metadata is added to the asset channel.
The deploy smoke must also verify public `Cache-Control` headers: mutable
release-channel pointers (`/`, `/health.json`, and
`/assets/<channel>/manifest.json`) stay `no-cache, must-revalidate`, while
immutable asset and profile release artifacts stay
`public, max-age=31536000, immutable`.

### Release-channel Cloudflare prerequisites

Before running a live binary or VM asset channel deploy, create or verify the
Cloudflare Pages project `capsem-release`, attach the `release.capsem.org`
custom domain, and configure `CLOUDFLARE_ACCOUNT_ID` plus
`CLOUDFLARE_API_TOKEN` in GitHub Actions secrets. `release-channel.yaml` fails
before deploy if either secret is missing, then smokes
`https://release.capsem.org/`, `/health.json`, and the channel manifest through
the public custom domain after Cloudflare publishes the generated site.

## Live release activation order

Use this order when turning the 1.4 release rails on. Do not skip ahead because
later steps depend on earlier public state being true.

1. Publish or merge the release-rail commits to `main`.
2. Wait for the expanded `pr-gate` to pass on `main`.
3. Require only `pr-gate` in branch protection or active rulesets.
4. Provision the `release.capsem.org` Cloudflare Pages project and DNS for the
   generated `target/release-channel/` artifact.
5. Run `uv run python scripts/check-remote-release-readiness.py`; continue only
   after unpublished commits, remote fail-closed `pr-gate` shape, branch
   protection, `release.capsem.org` DNS, public cache headers, and
   release-channel content all pass.
6. Run the manual VM asset workflow as a dry run and review the
   `asset-release-plan`, `asset-release-delta`, and `asset-channel-preview`
   artifacts. For metadata-only asset release changes, review
   `asset-release-delta` and `asset-channel-preview`; no `asset-release-plan`
   is expected because there are no immutable VM blobs to republish.
7. Run the tag-triggered binary release rail only from an immutable `vX.Y.Z`
   tag after confirming the tag does not already exist remotely.
8. Run the manual VM asset workflow live only after reviewing
   `asset-release-plan` when `asset_blobs_changed` is true, or reviewing the
   metadata-only delta and channel preview when only release-channel metadata
   changed; it must publish changed VM blobs, attest them, and deploy
   `release.capsem.org`.
9. Run installed update smokes for the signed macOS `.pkg`, Linux `.deb`, VM
   asset refresh, profile update path, and staged cross-surface update state.

Asset-channel blobs are arch-prefixed (`arm64-vmlinuz`,
`arm64-initrd.img`, `arm64-rootfs.erofs`, `arm64-obom.cdx.json`, and x86_64
equivalents). The v2 manifest keeps bare logical filenames inside each arch map.

## Disk Layouts

**Dev** (repo `assets/` dir -- logical names, per-arch subdirs):
```
assets/arm64/vmlinuz
assets/arm64/initrd.img
assets/arm64/rootfs.erofs
assets/manifest.json
```

**Installed** (`~/.capsem/assets/` -- flat, hash-based filenames):
```
manifest.json
vmlinuz-2c0bd752db929642
initrd-e5e910e9ab38b873.img
rootfs-89eb92b83534d9d0.erofs
```

Hash-based naming: `{stem}-{hash[..16]}{ext}`. Same hash = same file across versions = natural dedup.

## Boot-Time Resolution

1. **Dev mode**: Service detects arch subdirs, passes `--kernel assets/{arch}/vmlinuz` etc. to capsem-process
2. **Installed mode**: Service reads v2 manifest, resolves `ManifestV2::resolve(binary_version, arch, base_dir)` to get hash-based file paths, passes `--kernel`, `--initrd`, `--rootfs` individually to capsem-process
3. **Hash check at boot**: `VmConfig::builder().build()` verifies BLAKE3 against compile-time hashes if available

## Cleanup

`cleanup_unused_assets(base_dir, manifest)` removes hash-named files not referenced by any non-deprecated asset release. Also removes legacy `v*/` directories.
Existing VM pins are preserved by the VM pinning rail; deprecation blocks new
selection rather than rewriting running VMs.

## Common Issues

**Hash mismatch at boot**: Assets on disk don't match the hashes baked into the binary. Fix: `just shell` (repacks initrd, regenerates manifest, touches build.rs to force recompile).

**Hashes silently skipped**: If `build.rs` can't extract hashes (manifest missing, wrong format), `option_env!()` returns `None` and verification is skipped.
