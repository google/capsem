# Release-Channel Publication Contract

Read this reference before changing release-channel graph generation, binary
or runtime publication, channel switching, deployment, Cloudflare readiness,
evidence and attestation validation, or public cache headers.

The public asset channel is generated from that manifest with
`capsem-admin assets channel build`. Do not invent a separate release-channel
source tree or alternate manifest format. The generated deploy root is
`cache/target/release/distribution/`; the machine artifact is
`assets/<channel>/manifest.json` under that root, so the stable public URL is
`https://release.capsem.org/assets/stable/manifest.json`.
`capsem-admin` writes the machine channel artifacts only: root `channels.json`,
per-channel manifest JSON, runtime image/evidence files,
`_headers`, and `robots.txt`. The human release pages are built by the
`build_system/release_site/` Astro
app from those JSON files with
`CAPSEM_RELEASE_GRAPH=/path/to/cache/target/release/distribution CAPSEM_RELEASE_CHANNEL_DIST=/path/to/cache/target/release/distribution pnpm run
build:channel`, which overlays the root channel list and per-channel pages
into the same deploy root before channel validation or deployment.

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
5. The one `runtime` document owns the runtime images (kernel, initrd,
   rootfs per architecture), software inventory, OBOM evidence, and the
   optional `min_capsem_version`/`max_capsem_version`. It never advertises the
   selected Capsem binary. A channel that has published no runtime yet has no
   `runtime` key.

Immutable runtime image blobs are referenced by instantiated URLs in the
selected channel manifest. Public releases may store large blobs in GitHub
Releases, but the release graph must publish concrete URLs for each runtime
image artifact and evidence file, under
`/runtime/releases/<channel>/<revision>/<arch>/<file>`. When a local or corporate manifest is used,
the same update mechanism applies: `--manifest` must be a URL, with
`file:///absolute/path/to/manifest.json` for local fixtures and `https://...`
or `http://...` for hosted corporate channels.

The root channel catalog makes stable/nightly switching a manifest URL choice.
Stable can point at `https://release.capsem.org/assets/stable/manifest.json`
while nightly points at `https://release.capsem.org/assets/nightly/manifest.json`.
Publication dependencies are deliberately one-way. A stable publication is
self-contained and must never read, preserve, validate, or wait for nightly;
nightly may be absent or broken without blocking stable. A nightly publication
must resolve the latest good public stable graph, carry it byte-for-byte into
the generated distribution, and then add nightly so clients can always switch
back. If that stable baseline is unavailable or invalid, nightly fails closed.
Package postinstall and glow-up tests must use those URL-shaped inputs directly;
do not add package-time manifest converters or compatibility adapters for old
manifest shapes.
Updating the nightly runtime must change only the nightly `runtime` record and
matching digests; stable, packages, and per-binary inventory must stay
byte-for-byte unchanged. Use `min_capsem_version` on the runtime only when
runtime behavior requires a newer client.

Runtime publication is owned by:

```bash
just release-assets <channel> <source-commit>
```

That command calls `capsem-admin release`, which dispatches
`release-assets.yaml`. The shared `capsem-release-<channel>` lock is acquired
before the source manifest is read. The runtime workflow then resolves the
existing package by recorded digest, builds the runtime for arm64 and x86_64,
validates the pairing, and mutates only the channel's `runtime` entry. It never
builds a package and never edits another channel.

The runtime revision is `<workspace version>-<first 12 hex of the source
commit>`, so every released commit has its own identity even when nightly
re-releases at an unchanged workspace version. Images, software inventory,
OBOM and evidence are published under the immutable identity
`runtime-<channel>-<revision>`; reusing an identity for different bytes is
refused. This prevents the same revision label in stable and nightly from
aliasing or overwriting bytes.

When `min_capsem_version` is newer than the public package, the immutable
runtime publication is staged but not deployed. The following
`just release-binaries <channel> <source-commit>` resolves those exact staged digests, builds
packages only, runs the complete functional/native/glow-up proof, and activates
the completed pairing. The runtime bytes are not rebuilt.

The selected channel source manifest is the sole mutable authority. SBOM,
OBOM, existing attestations, and GitHub logs are the evidence; do not add a
parallel result or provenance file. Corporate manifest/runtime authoring also
goes through `capsem-admin`; corporations do not build Capsem binaries.

The deploy workflow runs `build_system/release_site/scripts/check-release-site-contract.py` against
`https://release.capsem.org` after Cloudflare publishes the generated site. That
Python validator reuses the remote release readiness contract and must validate
the root channel catalog, selected manifest, runtime
image/evidence files, package metadata, per-binary metadata,
BLAKE3/SHA-256 content, attestation references, and cache headers rather than
only checking that files exist. The deploy smoke rejects stale public HTML: the
root and channel pages must show the same generated timestamp, manifest URL,
manifest version, package inventory, per-binary inventory, runtime revision,
image artifact URLs, and evidence URLs as the fetched JSON
graph. It validates host SBOM and VM OBOM evidence document shape (SPDX 2.3 for
the host SBOM and CycloneDX for VM OBOMs). VM OBOM validation is provenance
validation, not only `bomFormat`: the document must declare
`capsem:evidence:scope=exported-rootfs`, contain Debian guest package purls, and
contain no `cdx:osquery:category` live-host inventory. It also validates
attestation scope, workflow, subjects, and predicate URLs against the published
host SBOM and VM OBOM evidence lists. VM asset attestations are incomplete unless
`github_attestations_vm_assets` is present and its `predicate_url` points at the
published VM OBOM evidence for the current asset release.
The deploy smoke must also verify public `Cache-Control` headers: mutable
release-channel pointers (`/`, `/channels.json`, and
`/assets/<channel>/manifest.json`) stay `no-cache, must-revalidate`, while
immutable asset and runtime release artifacts stay
`public, max-age=31536000, immutable`.

### Release-channel Cloudflare prerequisites

Before running a live binary or runtime channel deploy, create or verify the
Cloudflare Pages project serving `release.capsem.org`, attach the `release.capsem.org`
custom domain, and configure `CLOUDFLARE_ACCOUNT_ID` plus
`CLOUDFLARE_API_TOKEN` in GitHub Actions secrets. `release-channel.yaml` fails
before deploy if either secret is missing or
`build_system/scripts/web/check-cloudflare-pages-project.py` cannot see the Pages project through
the configured account/token, then runs `build_system/release_site/scripts/check-release-site-contract.py`
and smokes `https://release.capsem.org/`, `/channels.json`, and the channel
manifest through the public custom domain after Cloudflare publishes the
generated site. `release-channel-staging.yaml` proves this reusable deploy path
on a preview branch without invoking VM asset builds or package builders.

Asset-channel blobs are arch-prefixed (`arm64-vmlinuz`,
`arm64-initrd.img`, `arm64-rootfs.erofs`, `arm64-obom.cdx.json`,
`arm64-software-inventory.json`, and x86_64
equivalents). The v2 manifest keeps bare logical filenames inside each arch map.
