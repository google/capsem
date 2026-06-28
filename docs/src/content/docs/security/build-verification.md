---
title: Build Verification
description: Release attestation, SBOM, code signing, and notarization.
sidebar:
  order: 25
---

Capsem's release pipeline signs, notarizes, attests, and hash-verifies every artifact from source to installed binary.

## Release pipeline

```mermaid
graph LR
    A["Source<br/>(tagged commit)"] --> B["Build<br/>(per-arch)"]
    B --> C["Test<br/>(audit + coverage)"]
    C --> D["Codesign<br/>(Developer ID)"]
    D --> E["Notarize<br/>(Apple)"]
    E --> F["SBOM<br/>(SPDX 2.3)"]
    F --> G["Attest<br/>(SLSA + SBOM)"]
    G --> H["Publish manifest<br/>(BLAKE3 metadata)"]
    H --> I["Publish<br/>(GitHub release)"]
```

Every step is automated in `.github/workflows/release.yaml`. A preflight job validates signing credentials before any build starts.

## Code signing

All host binaries are codesigned with a Developer ID certificate. The `com.apple.security.virtualization` entitlement is required for Apple Virtualization.framework.

### Signed binaries

| Binary | Purpose | Entitlement |
|--------|---------|-------------|
| `capsem` | CLI client | `com.apple.security.virtualization` |
| `capsem-service` | Background daemon | `com.apple.security.virtualization` |
| `capsem-process` | Per-VM process | `com.apple.security.virtualization` |
| `capsem-mcp` | MCP server | `com.apple.security.virtualization` |
| `capsem-gateway` | HTTP gateway | `com.apple.security.virtualization` |
| `capsem-tray` | System tray | `com.apple.security.virtualization` |
| `Capsem.app` | Tauri desktop app | `com.apple.security.virtualization` |

### Development vs release signing

| Context | Signing | Command |
|---------|---------|---------|
| Development | Ad-hoc (`--sign -`) | `just build` (automatic) |
| Release | Developer ID certificate | `codesign --sign "$APPLE_SIGNING_IDENTITY" --entitlements entitlements.plist --force` |

Ad-hoc signing is sufficient for local development. The justfile handles this automatically on macOS.

## Notarization

Release builds are submitted to Apple for notarization, which scans for malware and validates the signature:

```
xcrun notarytool submit Capsem-$VERSION.pkg \
  --key $APPLE_API_KEY_PATH \
  --key-id $APPLE_API_KEY \
  --issuer $APPLE_API_ISSUER \
  --wait --timeout 30m
xcrun stapler staple Capsem-$VERSION.pkg
```

Stapling embeds the notarization ticket in the artifact so macOS can verify it offline.

## SBOM and OBOM

Host binaries publish a Software Bill of Materials using `cargo-sbom`:

```
cargo sbom --output-format spdx_json_2_3 > capsem-sbom.spdx.json
```

| Field | Value |
|-------|-------|
| Format | SPDX 2.3 JSON |
| Scope | All Rust crate dependencies |
| Published as | `capsem-sbom.spdx.json` in GitHub release |
| Attestation | SBOM attested against the macOS `.pkg` artifact |

VM base images publish an Operations Bill of Materials as CycloneDX JSON. CI
generates it with `cdxgen -t os` against the exported Linux rootfs before EROFS
cleanup, pins it in `manifest.json`, and publishes it with the profile assets.

| Field | Value |
|-------|-------|
| Format | CycloneDX OBOM JSON |
| Scope | Base Linux VM image only |
| Excludes | User session mutations, workspace writes, and post-boot state |
| Published as | `<arch>-obom.cdx.json` with profile assets |
| Integrity | BLAKE3 hash stored in the materialized profile |
| Runtime API | `GET /profiles/{profile_id}/info` and `GET /profiles/{profile_id}/obom` |

The profile OBOM descriptor records the OBOM file URL, BLAKE3 hash, size,
generator, generator version, and the rootfs BLAKE3 hash it describes. Runtime
routes expose the descriptor as profile evidence; local OBOM documents are
served only after size and BLAKE3 verification.

The per-architecture `build-ledger.log` is separate debug evidence. It records
the inputs that produced the rootfs, including rendered Dockerfiles, build
context hashes, EROFS settings, git/project version, profile root and
install-script inputs, and declared package config. It is not uploaded as the
release inventory and must not claim installed package state; installed
component names and versions come from the OBOM.

## SLSA attestation

Release artifacts receive [SLSA build provenance](https://slsa.dev/) attestation via `actions/attest-build-provenance@v4`:

| Artifact | Attestation |
|----------|-------------|
| `.pkg` (macOS installer) | Build provenance |
| `.deb` (Linux package) | Build provenance |
| `vmlinuz`, `initrd.img`, `rootfs.erofs`, `obom.cdx.json` (arm64) | VM asset build provenance |
| `vmlinuz`, `initrd.img`, `rootfs.erofs`, `obom.cdx.json` (x86_64) | VM asset build provenance |
| `.pkg` | SBOM (SPDX 2.3) |
| `<arch>-obom.cdx.json` | OBOM document, hash-pinned in `manifest.json` |

Attestations are published to the GitHub Attestations API and can be verified with `gh attestation verify`.
The VM `build-ledger.log` and `B3SUMS` outputs remain debug evidence unless a
future release intentionally publishes them as separate evidence artifacts.

## Asset integrity

VM assets (kernel, initrd, rootfs) are verified via BLAKE3 hashes at every stage
from build to boot. The checked-in profile is materialized into
`target/config/` before runtime, so the service boots from a generated profile
whose asset URLs, hashes, and sizes come directly from `assets/manifest.json`.

`assets/manifest.json` is generated through `capsem-admin manifest generate
<assets_dir>`. Release automation, local packaging, and corp custom builds use
that same admin command; lower-level manifest generation internals are not a
supported public path.

### Verification flow

```mermaid
graph TD
    A["Build assets<br/>capsem-admin manifest generate"] --> B["manifest.json<br/>(BLAKE3 hashes + sizes)"]
    B --> C["Release<br/>packages + arch-prefixed VM assets"]
    C --> D["Download<br/>profile/corp selected URL"]
    D --> E["Verify hashes<br/>BLAKE3 per-file check"]
    E --> F["Boot<br/>assets loaded from verified dir"]
```

### manifest.json schema

```json
{
  "format": 2,
  "assets": {
    "current": "2026.0627.1",
    "releases": {
      "2026.0627.1": {
        "min_binary": "1.0.0",
        "arches": {
          "arm64": {
            "vmlinuz": {"hash": "<64-char blake3>", "size": 8786432},
            "initrd.img": {"hash": "<64-char blake3>", "size": 996043},
            "rootfs.erofs": {"hash": "<64-char blake3>", "size": 978903040},
            "obom.cdx.json": {"hash": "<64-char blake3>", "size": 3499593}
          }
        }
      }
    }
  },
  "binaries": {
    "current": "1.3.1782582155",
    "releases": {
      "1.3.1782582155": {
        "min_assets": "2026.0627.1",
        "files": [
          {"name": "Capsem-1.3.1782582155.pkg", "sha256": "<sha256>"}
        ]
      }
    }
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `format` | integer | Manifest format; current format is `2` |
| `assets.current` | string | Current VM asset release id |
| `assets.releases.*.arches` | map | Arch -> logical asset names |
| `assets.releases.*.deprecated` | boolean | Release-history flag; deprecated VM asset releases remain auditable but are not selected for new sessions or downloads |
| `vmlinuz`, `initrd.img`, `rootfs.erofs`, `obom.cdx.json` | object | Bare logical filename with BLAKE3 hash and byte size |
| `binaries.current` | string | Current binary release id |
| `binaries.releases.*.files` | list | Published package filenames with SHA-256 metadata |

On GitHub Releases the VM files are arch-prefixed, for example
`arm64-rootfs.erofs`; inside the manifest they remain bare names under the
corresponding arch map.

### Hash verification

BLAKE3 hashes are computed in 256 KB chunks:

```rust
pub fn hash_file(path: &Path) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 { break; }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}
```

Validation rules:
- Hash must be exactly 64 hex characters
- Filenames must not contain `/`, `\`, or `..` (path traversal prevention)
- Version strings must not contain `..`, `/`, or `\`
- Empty releases are rejected

### Multi-version manifest

The manifest accumulates entries across releases. Each release merges its new
version entry with the previous manifest from the latest GitHub release. This
allows the asset service to download assets for any supported version.

## Manifest Role

`manifest.json` is release metadata: asset hashes, sizes, and version index.
It is published with the release alongside SBOM and provenance attestations.
Runtime trust comes from profile/corp-selected URLs plus BLAKE3 verification of
the downloaded bytes.

For a custom corp package, generate and verify the manifest from the built asset
directory before packaging:

```bash
capsem-admin manifest generate /path/to/assets --version 1.3.corp.1 --json
capsem-admin manifest check /path/to/assets/manifest.json --json
bash scripts/build-pkg.sh --manifest file:///path/to/assets/manifest.json ...
```

The installer moves that manifest into the installed service asset directory,
and status reports the installed manifest hash plus package provenance.
`--manifest` is URL-only so custom local manifests use `file://` and hosted
corporate channels use `https://` or `http://`.

## Supply chain controls

| Control | Implementation |
|---------|---------------|
| Rust toolchain | Stable, pinned via `dtolnay/rust-toolchain@stable` |
| Dependency audit | `cargo audit` in CI test stage |
| npm audit | `pnpm audit` in CI test stage |
| Docker base images | Resolved by the profile-derived Docker template rail |
| Compiler warnings | Treated as errors (`#[deny(warnings)]` in all crates) |
| Auditable builds | `cargo-auditable` embeds dependency info in binaries |
| Build context validation | `capsem.builder.doctor.check_source_files()` verifies completeness before release |
| Rootfs binary verification | Release pipeline checks all required guest binaries exist in rootfs before packaging |

### Required guest binaries

The release pipeline verifies these binaries exist in the rootfs before packaging:

| Binary | Purpose |
|--------|---------|
| `capsem-pty-agent` | PTY bridge and control channel |
| `capsem-net-proxy` | HTTPS proxy bridge |
| `capsem-mcp-server` | Guest MCP relay |
| `capsem-doctor` | In-VM diagnostics |
| `capsem-bench` | Performance benchmarks |
| `snapshots` | Snapshot management |
