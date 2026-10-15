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
| Channel discovery | `channels.json` | Rebuilding binaries or runtime images |
| Host install | Manifest `packages[]` | Rebuilding runtime images |
| VM runtime | Manifest `runtime` | Rebuilding host packages |

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
  runtime
    architectures[]
      software[]
      images[]
      evidence[]
```

The canonical ownership paths are:

```text
channels.json -> /assets/<channel>/manifest.json
channel -> packages -> binaries
channel -> runtime -> architectures -> software/images/evidence
```

The path tells readers who owns a fact. Package facts do not repeat in binary
records. Runtime image facts do not appear in channel summaries. If the owning
JSON object for a path does not contain a fact, the HTML page for that path
must not display that fact.

## Independent Version Surfaces

Manifest versions, package versions, and runtime revisions are independent.

A package release may change without changing the runtime revision or runtime images.
That is the fast binary-update rail.

A runtime revision may change without changing package versions.
That is the runtime asset rail. Every released commit has its own runtime
revision, `<workspace version>-<first 12 hex of the source commit>`, so a
nightly re-release at an unchanged workspace version never collides with an
earlier one.

The runtime may declare `min_capsem_version`; it must not select the current Capsem binary.
The channel selects the manifest. The manifest lists packages and the runtime.
The runtime only states the Capsem version range it supports.

Applications are not part of the release graph. They are OCI images published
by `images.yaml` to ghcr.io and resolved through the image catalog, on their own
cadence.

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
independent from Capsem package versions and runtime revisions. Human channel
lists display this manifest version, not the host package version or runtime
revision selected by that manifest.

Do not publish HMAC fields in the graph. SHA-256 is the compliance digest.
BLAKE3 is the fast content digest. Digests must be computed over bytes.
Repeated-character placeholders such as `1111...`, `aaaa...`, or `0000...` are
invalid release facts.

## Manifests

A manifest is a channel/version contract. It contains host install packages and
one VM runtime document:

```json
{
  "version": "1.0.2",
  "channel": "stable",
  "status": "current",
  "packages": [],
  "runtime": {}
}
```

`runtime` is absent (or null) for a channel that has published no runtime yet,
such as a binary-only first release.

The manifest must not use the legacy asset-channel shape as its public graph
shape:

```json
{
  "assets": {"current": "..."},
  "binaries": {"current": "..."}
}
```

That legacy shape is an internal compatibility input until the runtime selector
migrates. The public release graph uses packages and the runtime.

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

## Runtime

The runtime owns the VM images, evidence, software inventory, and Capsem
compatibility range. It is one kernel, initrd, and rootfs set per architecture,
built from `config/docker/image` and `guest/artifacts` with no other input:

```json
{
  "revision": "1.4.0-0123456789ab",
  "source_commit": "0123456789abcdef0123456789abcdef01234567",
  "status": "current",
  "min_capsem_version": "1.4.0",
  "architectures": [
    {
      "architecture": "arm64",
      "package_inventory_revision": "1.4.0-0123456789ab",
      "image_revision": "1.4.0-0123456789ab",
      "software": [],
      "images": [],
      "evidence": []
    }
  ]
}
```

`source_commit`, `min_capsem_version`, and `max_capsem_version` are optional.
The runtime has no `id`, `name`, `description`, or second `version`, and no
config references: no MCP, rule, package-list, or root-seed file is
published.

The runtime does not select a current Capsem binary. It may declare
`min_capsem_version` when it requires newer client behavior.

Forbidden runtime fields:

```text
current_binary
current_assets
asset_version
binary_version
```

## Software Inventory

Software inventory is runtime-owned image content. It must be complete for the
runtime image it describes and must be generated from the same image build
evidence as the image artifacts.

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
  "evidence": "/runtime/releases/stable/1.4.0-0123456789ab/arm64/software-inventory.json"
}
```

A runtime view may render software inventory only from the runtime JSON. It
must not display sample rows, inferred package names, or a partial hand-written
summary. A runtime with image artifacts must publish `software-inventory.json`;
missing inventory is a release-blocking generator failure, not a page-level
fallback.

## Runtime Images

Images are runtime-owned and architecture-scoped. Evidence attaches to the
image set it describes. Published files live under
`/runtime/releases/<channel>/<revision>/<architecture>/<file>`, and the GitHub
release that holds them is named `runtime-<channel>-<revision>`. Reusing a
publication identity for different bytes is refused.

```json
{
  "architectures": [
    {
      "architecture": "arm64",
      "images": [
        {
          "kind": "kernel",
          "name": "vmlinuz",
          "url": "/runtime/releases/stable/1.4.0-0123456789ab/arm64/vmlinuz",
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
          "url": "/runtime/releases/stable/1.4.0-0123456789ab/arm64/initrd.img",
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
          "url": "/runtime/releases/stable/1.4.0-0123456789ab/arm64/rootfs.erofs",
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
          "kind": "obom",
          "url": "/runtime/releases/stable/1.4.0-0123456789ab/arm64/obom.cdx.json",
          "bytes": 123,
          "digest": {
            "sha256": "...",
            "blake3": "..."
          },
          "status": "current"
        }
      ]
    }
  ]
}
```

Every architecture image set must include kernel, initrd, and rootfs artifacts
unless the runtime schema grows an explicit enum for a different boot mode. A
rootfs-only image set is incomplete and must fail the release gate. OBOM and
software inventory entries are not global evidence. They are runtime image
evidence.

## Page Contract

Pages render only their owning JSON:

| Page | Owning JSON |
| --- | --- |
| `/` | `channels.json` plus selected manifest links |
| `/channels/<channel>/` | `channels.<channel>` plus selected manifest |
| Runtime view of a channel | selected manifest `runtime` document |

If a string is not present in the owning JSON, the page must not display it as
a release fact. Labels such as table headers are allowed only for fields that
exist in the owning JSON shape.

Examples:

- A runtime view may show `min_capsem_version`; it must not show current
  binary state.
- A channel page may show manifest records, package rows, package-owned
  binaries, and the runtime revision. It must not show `Evidence`, `Host SBOM`,
  `VM OBOM`, runtime image artifacts, software inventory, or asset release
  history sections.
- No page should show HMAC columns because the graph does not publish HMAC.

## Release Gates

Release output tests must verify:

1. Every channel manifest URL resolves and its SHA-256/BLAKE3 match
   `channels.json`.
2. `channels.json` exposes exactly one public manifest URL per channel:
   `/assets/<channel>/manifest.json`.
3. No public graph carries the retired `profiles` key; the channel validator
   rejects it.
4. Every package has bytes, SHA-256, BLAKE3, and package-owned binaries.
5. Every binary has installed path, version, bytes, SHA-256, BLAKE3, and SBOM
   component.
6. No digest object contains HMAC.
7. No digest is a repeated-character placeholder.
8. The runtime publishes no config files.
9. Every runtime architecture includes kernel, initrd, and rootfs.
10. Every runtime image/evidence URL resolves and its bytes, SHA-256, and
   BLAKE3 match.
11. Every runtime software inventory entry is complete, hashed, and points at
   the generated `software-inventory.json` evidence artifact.
12. Runtime views contain only runtime-owned facts.
13. Channel pages contain only channel and manifest facts.
14. Stable and nightly may select different manifests and runtime revisions
    without mutating each other.
