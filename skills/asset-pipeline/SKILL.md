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
| Asset channel human site | `release-site/` Astro app, built from `target/release-channel/` JSON |
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
`capsem-admin` writes the machine channel artifacts only: root `channels.json`,
per-channel manifest JSON, profile-owned image/config/evidence files,
`_headers`, and `robots.txt`. The human release pages are built by the
`release-site/` Astro
app from those JSON files with
`CAPSEM_RELEASE_CHANNEL_DIST=/path/to/target/release-channel pnpm run
build:channel`, which overlays the root channel list, per-channel pages, and
per-profile pages into the same deploy root before channel validation or
deployment.

The graph hierarchy is strict:

1. `channels.json` lists all channels and all versioned manifest records for
   each channel.
2. Each manifest record has one status enum value: `current`, `supported`,
   `deprecated`, or `revoked`. Revoked records remain auditable but runtime
   selection never chooses them. A record that is no longer served is simply
   absent.
3. Each manifest record carries SHA-256 and BLAKE3 digests for the selected
   manifest JSON. Do not publish HMAC fields.
4. Each manifest keeps package artifacts separate from per-binary inventory.
   Packages are delivery containers; binaries are the executable files inside
   those packages and must carry SHA-256, BLAKE3, version, package
   provenance, and SBOM component reference.
5. Profiles own profile images, config files, software inventory, ABOM/OBOM
   evidence, and `min_capsem_version`. Profiles never advertise the selected
   Capsem binary; they only declare the minimum Capsem version needed to use
   that profile.

Immutable profile image blobs are referenced by instantiated URLs in the
selected channel manifest. Public releases may store large blobs in GitHub
Releases, but the release graph must publish concrete URLs for each profile
image artifact and evidence file. When a local or corporate manifest is used,
the same update mechanism applies: `--manifest` must be a URL, with
`file:///absolute/path/to/manifest.json` for local fixtures and `https://...`
or `http://...` for hosted corporate channels.

The root channel catalog makes stable/nightly switching a manifest URL choice.
Stable can point at `https://release.capsem.org/assets/stable/manifest.json`
while nightly points at `https://release.capsem.org/assets/nightly/manifest.json`.
Package postinstall and glow-up tests must use those URL-shaped inputs directly;
do not add package-time manifest converters or compatibility adapters for old
manifest shapes.
Updating the co-work nightly profile image/config must change only the nightly
channel/profile records and matching digests; stable, packages, per-binary
inventory, and other profiles must stay byte-for-byte unchanged. Use
`min_capsem_version` on a profile only when profile behavior requires a newer
client.

The manual asset workflow is `.github/workflows/release-assets.yaml`. It should
remain explicit/manual. For `dry_run=false`, it first verifies that the
configured `CLOUDFLARE_ACCOUNT_ID` and `CLOUDFLARE_API_TOKEN` can see the
Pages project serving `release.capsem.org`, so a bad release-site binding fails before VM image
builds, immutable GitHub asset publication, or provenance attestation. It should
build VM assets, publish changed blobs to an immutable
`assets-v<asset-version>` GitHub Release, attest the arch-prefixed `vmlinuz`,
`initrd.img`, `rootfs.erofs`, `obom.cdx.json`, and
`software-inventory.json` subjects, write `asset_base`
into the channel manifest, run the Astro release-site build against the
generated channel data, upload `target/release-channel/` without VM blobs as
the `asset-channel-preview` artifact, and call
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
The deploy workflow runs `scripts/check-release-site-contract.py` against
`https://release.capsem.org` after Cloudflare publishes the generated site. That
Python validator reuses the remote release readiness contract and must validate
the root channel catalog, selected manifest, profile-owned
image/config/evidence files, package metadata, per-binary metadata,
BLAKE3/SHA-256 content, attestation references, and cache headers rather than
only checking that files exist. The deploy smoke rejects stale public HTML: the
root and channel pages must show the same generated timestamp, manifest URL,
manifest version, package inventory, per-binary inventory, profile revision,
image artifact URLs, and evidence URLs as the fetched JSON
graph. It validates host SBOM and VM OBOM evidence document shape (SPDX 2.3 for
the host SBOM and CycloneDX for VM OBOMs), plus attestation scope, workflow,
subjects, and predicate URLs against the published host SBOM and VM OBOM
evidence lists. VM asset attestations are incomplete unless
`github_attestations_vm_assets` is present and its `predicate_url` points at the
published VM OBOM evidence for the current asset release.
The deploy smoke must also verify public `Cache-Control` headers: mutable
release-channel pointers (`/`, `/channels.json`, and
`/assets/<channel>/manifest.json`) stay `no-cache, must-revalidate`, while
immutable asset and profile release artifacts stay
`public, max-age=31536000, immutable`.

### Release-channel Cloudflare prerequisites

Before running a live binary or VM asset channel deploy, create or verify the
Cloudflare Pages project serving `release.capsem.org`, attach the `release.capsem.org`
custom domain, and configure `CLOUDFLARE_ACCOUNT_ID` plus
`CLOUDFLARE_API_TOKEN` in GitHub Actions secrets. `release-channel.yaml` fails
before deploy if either secret is missing or
`scripts/check-cloudflare-pages-project.py` cannot see the Pages project through
the configured account/token, then runs `scripts/check-release-site-contract.py`
and smokes `https://release.capsem.org/`, `/channels.json`, and the channel
manifest through the public custom domain after Cloudflare publishes the
generated site. Live VM asset releases use the same project preflight before
the expensive asset build matrix starts.

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
6. Run `.github/workflows/release-channel-staging.yaml` against the Cloudflare
   Pages staging branch. It builds a deterministic fixture, deploys the
   generated channel through `.github/workflows/release-channel.yaml`, and
   validates the same release-channel contract without invoking `build-assets`
   or binary package builds.
7. Run the manual VM asset workflow as a dry run and review the
   `asset-release-plan`, `asset-release-delta`, and `asset-channel-preview`
   artifacts. For metadata-only asset release changes, review
   `asset-release-delta` and `asset-channel-preview`; no `asset-release-plan`
   is expected because there are no immutable VM blobs to republish.
8. Run `.github/workflows/release-binary-staging.yaml` and review the
   `binary-channel-dry-run-bundle` artifact. It records deterministic fake host
   package and `capsem-sbom.spdx.json` metadata into a copy of the live asset
   manifest, builds the release-site preview, and writes `proof.json` showing
   VM asset metadata was not changed. This is the safe binary dry-run path; do
   not add `workflow_dispatch` to the real tag-triggered `release.yaml`.
9. Run the tag-triggered binary release rail only from an immutable `vX.Y.Z`
   tag after confirming the tag does not already exist remotely.
10. Run the manual VM asset workflow live only after reviewing
   `asset-release-plan` when `asset_blobs_changed` is true, or reviewing the
   metadata-only delta and channel preview when only release-channel metadata
   changed; it must publish changed VM blobs, attest them, and deploy
   `release.capsem.org`.
11. Run installed update smokes for the signed macOS `.pkg`, Linux `.deb`, VM
   asset refresh, profile update path, and staged cross-surface update state.

Asset-channel blobs are arch-prefixed (`arm64-vmlinuz`,
`arm64-initrd.img`, `arm64-rootfs.erofs`, `arm64-obom.cdx.json`,
`arm64-software-inventory.json`, and x86_64
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
manifest-origin.json
vmlinuz-2c0bd752db929642
initrd-e5e910e9ab38b873.img
rootfs-89eb92b83534d9d0.erofs
```

Native packages do not carry `assets/manifest.json`. They carry
`manifest-origin.json` with the selected channel or corp manifest URL, and
postinstall runs `capsem update --assets --manifest <URL>` to write the live
installed manifest plus any missing profile image assets.

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
