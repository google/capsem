---
title: Release Output Contract
description: Public release graph shape and invariants for release.capsem.org.
sidebar:
  order: 36
---

`release.capsem.org` publishes a release graph. The JSON files are the source
of truth. The HTML pages are views over those files and must not invent fields,
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
      url -> /manifests/<channel>/<manifest-version>/manifest.json
      digest.sha256
      digest.blake3

manifest.json
  packages[]
    binaries[]
  profiles.<profile>
    config[]
    images.<architecture>
      artifacts[]
      evidence[]

profiles/releases/<catalog-version>/catalog.json
  profiles[]
```

The path tells readers who owns a fact. Package facts do not repeat in binary
records. Profile image facts do not appear in channel summaries unless the
profile JSON also contains them.

## Channels

`/channels.json` lists all public channels and the manifest history for each
channel. Examples are `stable` and `nightly`.

Each manifest record has exactly one status:

```text
current | supported | deprecated | revoked
```

There is no `removed` status. Removing a manifest means omitting it from the
channel list. Records that remain in `channels.json` remain auditable.

Each manifest record must include:

```json
{
  "version": "1.4.0",
  "status": "current",
  "url": "/manifests/stable/1.4.0/manifest.json",
  "digest": {
    "sha256": "...",
    "blake3": "..."
  }
}
```

The digest is over the referenced manifest bytes. Do not publish HMAC fields in
the graph. SHA-256 is the compliance digest. BLAKE3 is the fast content digest.

## Manifests

A manifest is a channel/version contract. It contains host install packages and
profile asset references:

```json
{
  "version": "1.4.0",
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

## Profiles

Profiles own config files, profile images, evidence, software inventory, and
minimum Capsem compatibility:

```json
{
  "id": "code",
  "name": "Code",
  "revision": "2026.07.02.1-stable",
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

## Profile Images

Images are profile-owned and architecture-scoped. Evidence attaches to the
image set it describes:

```json
{
  "images": {
    "arm64": {
      "artifacts": [
        {
          "kind": "rootfs",
          "name": "rootfs.erofs",
          "url": "/profiles/releases/2026.07.02.1-stable/code/arm64/rootfs.erofs",
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
          "url": "/profiles/releases/2026.07.02.1-stable/code/arm64/abom.cdx.json",
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

ABOM and OBOM entries are not global evidence. They are profile image evidence.

## Profile Catalog

The profile catalog is an immutable snapshot:

```text
/profiles/releases/<catalog-version>/catalog.json
```

The catalog profile ids and revisions must match the selected manifest. Its
BLAKE3 hash in `channels.json` must equal the catalog bytes. The catalog must
not add fields that are forbidden on profiles.

## Page Contract

Pages render only their owning JSON:

| Page | Owning JSON |
| --- | --- |
| `/` | `channels.json` plus the selected manifest/catalog |
| `/channels/<channel>/` | `channels.<channel>` plus selected manifest/catalog |
| `/channels/<channel>/profiles/<profile>/` | selected manifest profile entry |

If a string is not present in the owning JSON, the page must not display it as
a release fact. Labels such as table headers are allowed only for fields that
exist in the owning JSON shape.

Examples:

- A profile page may show `min_capsem_version`; it must not show current
  binary state.
- A channel page may show package rows and package-owned binaries; it must not
  show detached profile image evidence.
- No page should show HMAC columns because the graph does not publish HMAC.

## Release Gates

Release output tests must verify:

1. Every channel manifest URL resolves and its SHA-256/BLAKE3 match
   `channels.json`.
2. Every profile catalog URL resolves and its BLAKE3 matches `channels.json`.
3. Catalog profiles match manifest profiles by id and revision.
4. Every package has bytes, SHA-256, BLAKE3, and package-owned binaries.
5. Every binary has installed path, bytes, SHA-256, BLAKE3, and SBOM component.
6. No digest object contains HMAC.
7. Every profile config/image/evidence URL resolves and its bytes, SHA-256,
   and BLAKE3 match.
8. Profile pages contain only profile-owned facts.
9. Channel pages contain only channel and manifest facts.
10. Stable and nightly may select different manifests and profile revisions
    without mutating each other.
