# Manifest and Storage Contract

Read this reference before changing the v2 manifest schema, digest generation
or enrichment, corporate/local manifest inputs, installed asset layout, package
manifest metadata, or hash-based cache naming.

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
            "vmlinuz": { "hash": "<64-char blake3>", "sha256": "<64-char sha256>", "size": 7797248 },
            "initrd.img": { "hash": "...", "sha256": "...", "size": 2270154 },
            "rootfs.erofs": { "hash": "...", "sha256": "...", "size": 454230016 }
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

Digest ownership starts at that asset build/ingest boundary. Stream each asset
once there and persist both BLAKE3 identity and SHA-256 compatibility evidence
in its manifest entry. Release-channel assembly trusts complete recorded
digests for remote immutable blobs; it must not reopen the same rootfs merely
to render stable and nightly graphs. A legacy current entry missing SHA-256 may
be hydrated once from its matching current file, but historical releases must
never be compared with the flat current `assets/<arch>/<logical-name>` path.
When local channel output copies blobs, compute and validate both digests in
the copy stream and reuse that result for graph rendering. Digest enrichment
alone does not mint a new asset version; only BLAKE3/size identity changes do.

## Disk Layouts

**Dev** (repo `cache/target/assets/` dir -- logical names, per-arch subdirs):
```
cache/target/assets/arm64/vmlinuz
cache/target/assets/arm64/initrd.img
cache/target/assets/arm64/rootfs.erofs
cache/target/assets/manifest.json
```

**Installed** (`~/.capsem/assets/` -- flat, hash-based filenames):
```
manifest.json
manifest-metadata.json
vmlinuz-2c0bd752db929642
initrd-e5e910e9ab38b873.img
rootfs-89eb92b83534d9d0.erofs
```

Native packages do not carry the repository's `cache/target/assets/manifest.json`. They carry
`manifest-metadata.json` with the selected channel or corp manifest URL, and
postinstall runs `capsem update --assets --manifest <URL>` to write the live
installed manifest plus any missing profile image assets.

Hash-based naming: `{stem}-{hash[..16]}{ext}`. Same hash = same file across versions = natural dedup.
