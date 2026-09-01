# Release-Channel Publication Contract

Read this reference before changing release-channel graph generation, binary
or profile publication, channel switching, deployment, Cloudflare readiness,
evidence and attestation validation, or public cache headers.

The public asset channel is generated from that manifest with
`capsem-admin assets channel build`. Do not invent a separate release-channel
source tree or alternate manifest format. The generated deploy root is
`cache/target/release/distribution/`; the machine artifact is
`assets/<channel>/manifest.json` under that root, so the stable public URL is
`https://release.capsem.org/assets/stable/manifest.json`.
`capsem-admin` writes the machine channel artifacts only: root `channels.json`,
per-channel manifest JSON, profile-owned image/config/evidence files,
`_headers`, and `robots.txt`. The human release pages are built by the
`build_system/release_site/` Astro
app from those JSON files with
`CAPSEM_RELEASE_GRAPH=/path/to/cache/target/release/distribution CAPSEM_RELEASE_CHANNEL_DIST=/path/to/cache/target/release/distribution pnpm run
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
Publication dependencies are deliberately one-way. A stable publication is
self-contained and must never read, preserve, validate, or wait for nightly;
nightly may be absent or broken without blocking stable. A nightly publication
must resolve the latest good public stable graph, carry it byte-for-byte into
the generated distribution, and then add nightly so clients can always switch
back. If that stable baseline is unavailable or invalid, nightly fails closed.
Package postinstall and glow-up tests must use those URL-shaped inputs directly;
do not add package-time manifest converters or compatibility adapters for old
manifest shapes.
Updating the co-work nightly profile image/config must change only the nightly
channel/profile records and matching digests; stable, packages, per-binary
inventory, and other profiles must stay byte-for-byte unchanged. Use
`min_capsem_version` on a profile only when profile behavior requires a newer
client.

Profile publication is owned by:

```bash
just release-profile <channel> <profile> <source-commit>
```

That command calls `capsem-admin release`. The shared
`capsem-release-<channel>` lock is acquired before the source manifest is read.
The profile workflow then resolves the existing package by recorded digest,
builds exactly the selected channel/profile for arm64 and x86_64, validates the
pairing, and mutates only that profile entry. It never builds a package and
never edits another profile or channel.

Profile config, images, software inventory, OBOM, evidence, and revision are
published under an immutable identity containing channel and profile identity.
This prevents the same profile/revision label in stable and nightly from
aliasing or overwriting bytes.

When `min_capsem_version` is newer than the public package, the immutable
profile publication is staged but not deployed. The following
`just release-binaries <channel> <source-commit>` resolves those exact staged digests, builds
packages only, runs the complete functional/native/glow-up proof, and activates
the completed pairing. The profile bytes are not rebuilt.

The selected channel source manifest is the sole mutable authority. SBOM,
OBOM, existing attestations, and GitHub logs are the evidence; do not add a
parallel result or provenance file. Corporate manifest/profile authoring also
goes through `capsem-admin`; corporations do not build Capsem binaries.

The deploy workflow runs `build_system/release_site/scripts/check-release-site-contract.py` against
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
immutable asset and profile release artifacts stay
`public, max-age=31536000, immutable`.

### Release-channel Cloudflare prerequisites

Before running a live binary or profile channel deploy, create or verify the
Cloudflare Pages project serving `release.capsem.org`, attach the `release.capsem.org`
custom domain, and configure `CLOUDFLARE_ACCOUNT_ID` plus
`CLOUDFLARE_API_TOKEN` in GitHub Actions secrets. `release-channel.yaml` fails
before deploy if either secret is missing or
`build_system/scripts/web/check-cloudflare-pages-project.py` cannot see the Pages project through
the configured account/token, then runs `build_system/release_site/scripts/check-release-site-contract.py`
and smokes `https://release.capsem.org/`, `/channels.json`, and the channel
manifest through the public custom domain after Cloudflare publishes the
generated site. `release-channel-staging.yaml` proves this reusable deploy path
on a preview branch without invoking profile builders or package builders.

Asset-channel blobs are arch-prefixed (`arm64-vmlinuz`,
`arm64-initrd.img`, `arm64-rootfs.erofs`, `arm64-obom.cdx.json`,
`arm64-software-inventory.json`, and x86_64
equivalents). The v2 manifest keeps bare logical filenames inside each arch map.
