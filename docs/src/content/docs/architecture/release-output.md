---
title: Release Output Contract
description: Public release graph shape and invariants for release.capsem.org.
sidebar:
  order: 36
---

`release.capsem.org` publishes a release graph. The JSON files are the source of truth.
The HTML pages are views over those files and must not invent fields,
sections, statuses, hashes, or URLs that are absent from the JSON object that
owns that page.

## Ownership Model

The release graph has three independent rails:

| Rail | Owner in the graph | May change without |
| --- | --- | --- |
| Channel discovery | `channels.json` | Rebuilding binaries or profile images |
| Host install | Manifest `packages[]` | Rebuilding profile images |
| Profile assets | Manifest `profiles{}` | Rebuilding host packages |

The graph is hierarchical:

```text
channels.json
  channels.<channel>
    manifests[]
      url -> /assets/<channel>/manifest.json
      digest.sha256
      digest.blake3

assets/<channel>/manifest.json
  packages[]
    binaries[]
  profiles.<profile>
    config[]
    images.<architecture>
      artifacts[]
      evidence[]
```

The canonical ownership paths are:

```text
channels.json -> /assets/<channel>/manifest.json
channel -> packages -> binaries
channel -> profiles -> architecture -> config/software/images
```

The path tells readers who owns a fact. Package facts do not repeat in binary
records. Profile image facts do not appear in channel summaries. If the owning
JSON object for a path does not contain a fact, the HTML page for that path
must not display that fact.

## Independent Version Surfaces

Manifest versions, package versions, profile revisions, and profile image revisions are independent.

A package release may change without changing profile revisions or profile images.
That is the fast binary-update rail.

A profile revision may change without changing package versions or other profiles.
That is the profile/config/software rail.

A profile image revision may change for one profile and architecture without changing other profiles, other architectures, or packages.

A profile may declare `min_capsem_version`; it must not select the current Capsem binary.
The channel selects the manifest. The manifest lists packages and profiles.
Profiles only state the minimum Capsem version they require.

## Channels

`/channels.json` lists all public channels and the manifest history for each
channel. Examples are `stable` and `nightly`.

All release status fields use the same enum:

```text
current | supported | deprecated | revoked
```

Each manifest record has exactly one status:

```text
current | supported | deprecated | revoked
```

There is no `removed` status. Removing a manifest means omitting it from the
channel list. Records that remain in `channels.json` remain auditable.

Each manifest record must include:

```json
{
  "version": "1.0.2",
  "status": "current",
  "url": "/assets/stable/manifest.json",
  "digest": {
    "sha256": "...",
    "blake3": "..."
  }
}
```

The digest is over the current `/assets/<channel>/manifest.json` bytes. There
is only one public manifest URL per channel. Historical manifest records remain
in `channels.json` for auditability, but they must not create alternate public
manifest URLs that compete with `/assets/<channel>/manifest.json`.

The manifest record `version` is the manifest contract version. It is
independent from Capsem package versions, profile revisions, and profile image
revisions. Human channel lists display this manifest version, not the host
package version or profile revision selected by that manifest.

Do not publish HMAC fields in the graph. SHA-256 is the compliance digest.
BLAKE3 is the fast content digest. Digests must be computed over bytes.
Repeated-character placeholders such as `1111...`, `aaaa...`, or `0000...` are
invalid release facts.

## Manifests

A manifest is a channel/version contract. It contains host install packages and
profile asset references:

```json
{
  "version": "1.0.2",
  "status": "current",
  "packages": [],
  "profiles": {}
}
```

The manifest must not use the legacy asset-channel shape as its public graph
shape:

```json
{
  "assets": {"current": "..."},
  "binaries": {"current": "..."}
}
```

That legacy shape is an internal compatibility input until the runtime selector
migrates. The public release graph uses packages and profiles.

## Packages And Binaries

Packages are host delivery containers such as `.pkg` and `.deb`. Binaries are
executables inside a package. The package owns its binaries:

```json
{
  "id": "macos-pkg-arm64",
  "kind": "macos_pkg",
  "name": "Capsem-1.4.0-arm64.pkg",
  "url": "/packages/stable/1.4.0/Capsem-1.4.0-arm64.pkg",
  "bytes": 123,
  "digest": {
    "sha256": "...",
    "blake3": "..."
  },
  "binaries": [
    {
      "name": "capsem",
      "installed_path": "/usr/local/bin/capsem",
      "bytes": 456,
      "digest": {
        "sha256": "...",
        "blake3": "..."
      },
      "sbom_component_ref": "SPDXRef-File-capsem"
    }
  ]
}
```

Do not repeat the package name on every binary. If a flat binary index is ever
needed for search, it is a derived index, not the canonical manifest shape.

The channel page renders package target rows from the selected manifest and
links to package detail pages. It must not flatten `packages[].binaries[]` into
a global channel binary table. Package detail pages are the owner view for
contained binaries, installed paths, binary hashes, SBOM component references,
and package evidence.

Package rows must have a download URL, byte count, SHA-256, BLAKE3, and package
SBOM evidence. Binary rows must be nested under packages in JSON and must
include an installed path, byte count, SHA-256, BLAKE3, and SBOM component
reference. `not published` and `unknown` are not valid values for a package or
binary row that is present in the manifest.

## Profiles

Profiles own config files, profile images, evidence, software inventory, and
minimum Capsem compatibility:

```json
{
  "id": "code",
  "name": "Code",
  "revision": "1.0.0-stable.20260702",
  "min_capsem_version": "1.4.0",
  "software": [],
  "config": [],
  "images": {}
}
```

Profiles do not select a current Capsem binary. A profile may declare
`min_capsem_version` when it requires newer client behavior.

Forbidden profile fields:

```text
current_binary
current_assets
asset_version
binary_version
```

## Software Inventory

Software inventory is profile-owned image content. It must be complete for the
profile image it describes and must be generated from the same profile/image
build evidence as the image artifacts.

Every software entry must include:

```json
{
  "name": "python",
  "version": "3.12.11",
  "source": "apt",
  "architecture": "arm64",
  "digest": {
    "sha256": "...",
    "blake3": "..."
  },
  "evidence": "/assets/releases/1.0.0-stable.20260702/arm64-software-inventory.json"
}
```

The profile page may render software inventory only from the profile JSON. It
must not display sample rows, inferred package names, or a partial hand-written
summary. Release profiles with image artifacts must publish
`software-inventory.json`; missing inventory is a release-blocking generator
failure, not a page-level fallback.

## Config Files

Config files are profile-owned and must be generated from the profile source
directory, not hand-written into the release page. For the built-in profiles,
the profile release must publish every file that defines the profile contract:

```text
profile.toml
mcp.json
enforcement.toml
detection.yaml
apt-packages.txt
python-requirements.txt
npm-packages.txt
build.sh
tips.txt
root.manifest.json
```

The config list may include additional files declared by `profile.toml`, but it
must not silently omit one of the files above when that file exists in
`config/profiles/<profile>/`. Every config entry must include `kind`, `path`,
`url`, `bytes`, and a `digest` object with `sha256` and `blake3`.

## Profile Images

Images are profile-owned and architecture-scoped. Evidence attaches to the
image set it describes:

```json
{
  "images": {
    "arm64": {
      "artifacts": [
        {
          "kind": "kernel",
          "name": "vmlinuz",
          "url": "/profiles/releases/1.0.0-stable.20260702/code/arm64/vmlinuz",
          "bytes": 123,
          "digest": {
            "sha256": "...",
            "blake3": "..."
          },
          "status": "current"
        },
        {
          "kind": "initrd",
          "name": "initrd.img",
          "url": "/profiles/releases/1.0.0-stable.20260702/code/arm64/initrd.img",
          "bytes": 123,
          "digest": {
            "sha256": "...",
            "blake3": "..."
          },
          "status": "current"
        },
        {
          "kind": "rootfs",
          "name": "rootfs.erofs",
          "url": "/profiles/releases/1.0.0-stable.20260702/code/arm64/rootfs.erofs",
          "bytes": 123,
          "digest": {
            "sha256": "...",
            "blake3": "..."
          },
          "status": "current"
        }
      ],
      "evidence": [
        {
          "kind": "abom",
          "url": "/profiles/releases/1.0.0-stable.20260702/code/arm64/abom.cdx.json",
          "bytes": 123,
          "digest": {
            "sha256": "...",
            "blake3": "..."
          },
          "status": "current"
        }
      ]
    }
  }
}
```

Every architecture image set must include kernel, initrd, and rootfs artifacts
unless the profile schema grows an explicit enum for a different boot mode. A
rootfs-only image set is incomplete and must fail the release gate. ABOM and
OBOM entries are not global evidence. They are profile image evidence.

## Page Contract

Pages render only their owning JSON:

| Page | Owning JSON |
| --- | --- |
| `/` | `channels.json` plus selected manifest links |
| `/channels/<channel>/` | `channels.<channel>` plus selected manifest |
| `/channels/<channel>/profiles/<profile>/` | selected manifest profile entry |

If a string is not present in the owning JSON, the page must not display it as
a release fact. Labels such as table headers are allowed only for fields that
exist in the owning JSON shape.

Examples:

- A profile page may show `min_capsem_version`; it must not show current
  binary state.
- A channel page may show manifest records, package rows, package-owned
  binaries, and profile references. It must not show `Evidence`, `Host SBOM`,
  `VM OBOM`, profile image artifacts, software inventory, or asset release
  history sections.
- No page should show HMAC columns because the graph does not publish HMAC.

## Release Gates

Release output tests must verify:

1. Every channel manifest URL resolves and its SHA-256/BLAKE3 match
   `channels.json`.
2. `channels.json` exposes exactly one public manifest URL per channel:
   `/assets/<channel>/manifest.json`.
3. No public graph or page exposes a profile catalog release primitive.
4. Every package has bytes, SHA-256, BLAKE3, and package-owned binaries.
5. Every binary has installed path, version, bytes, SHA-256, BLAKE3, and SBOM
   component.
6. No digest object contains HMAC.
7. No digest is a repeated-character placeholder.
8. Every profile config file required by the profile source is published.
9. Every profile image architecture includes kernel, initrd, and rootfs.
10. Every profile config/image/evidence URL resolves and its bytes, SHA-256,
   and BLAKE3 match.
11. Every profile software inventory entry is complete, hashed, and points at
   the generated `software-inventory.json` evidence artifact.
12. Profile pages contain only profile-owned facts.
13. Channel pages contain only channel and manifest facts.
14. Stable and nightly may select different manifests and profile revisions
    without mutating each other.
