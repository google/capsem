# T1: Image Builder and Manifest Compatibility

## Objective

Keep VM asset manifests compatible across binary releases and local rebuilds.
The same-day asset version, two-architecture manifest maps, rootfs contents,
and cleanup behavior must be deterministic enough that CI and local repacks
cannot silently publish or boot the wrong image.

## Owned Files

- `.github/workflows/release.yaml`
- `src/capsem/builder/docker.py`
- `scripts/gen_manifest.py`
- `justfile`
- `crates/capsem-core/src/asset_manager.rs`
- `crates/capsem-core/src/manifest_compat.rs`
- `tests/test_docker.py`
- `tests/test_gen_manifest.py`
- `tests/test_release_workflow.py`
- `tests/capsem-build-chain/*`
- `tests/capsem-rootfs-artifacts/*`
- `docs/src/content/docs/architecture/asset-pipeline.md`

## Findings

- [P1] Release manifest population overwrites the generated binary release
  entry with only `version` and `files`, dropping `date`, `deprecated`, and
  `min_assets`.
- [P1] `generate_checksums()` always emits `YYYY.MMDD.1`, while
  `scripts/gen_manifest.py` increments same-day patches.
- [P2] `_pack-initrd` regenerates `B3SUMS` for only the host arch and rewrites
  the top-level manifest from that partial file.
- [P2] `cleanup_unused_assets()` skips directories, so stale hash-named files
  under `$assets/{arch}/` and legacy `v1.0.*` dirs remain.
- [P1] Release rootfs validation runs in `build-app-linux`, whose job is
  `continue-on-error`, so failed squashfs validation may not block
  `create-release`.
- [P2] Mounted-rootfs validation omits canonical guest binaries such as
  `capsem-dns-proxy` and `capsem-sysutil`.
- [P3] Asset layout docs/comments still describe old v1 layouts.

## Task List

### T1.1 Preserve Binary Compatibility Metadata

- [ ] Preserve existing `min_assets`, `date`, and `deprecated` fields when
  adding binary file metadata in release manifest population.
- [ ] Add a regression fixture where create-release updates binary file
  metadata without changing the selected asset release for older binaries.
- [ ] Assert `ManifestV2::pick_asset_version` sees the expected `min_assets`
  after release manifest mutation.

### T1.2 Unify Asset Version Generation

- [ ] Make `generate_checksums()` share the same same-day patch increment
  behavior as `scripts/gen_manifest.py`.
- [ ] Move same-day patch selection into a shared helper or duplicate it with
  tests comparing both producers against an existing same-day manifest.
- [ ] Add a collision test for two same-day full asset builds.

### T1.3 Safe Local Initrd Repack

- [ ] Change `_pack-initrd` manifest regeneration so it updates only the host
  arch entry while preserving other arch maps, or recomputes all arch `B3SUMS`
  entries when both arch dirs exist.
- [ ] Add/adjust tests that assert two-arch manifests survive local repacks.
- [ ] Ensure local repack does not silently drop x86_64 maps on arm64 hosts or
  arm64 maps on x86_64 hosts.

### T1.4 Asset Cleanup

- [ ] Teach `cleanup_unused_assets()` to traverse arch subdirectories.
- [ ] Remove legacy `v1.0.*` directories.
- [ ] Add adversarial cleanup fixtures that include live hash files, stale hash
  files, arch subdirs, and legacy dirs.
- [ ] Assert cleanup never deletes a referenced non-deprecated asset.

### T1.5 Rootfs Validation as a Hard Gate

- [ ] Move squashfs/rootfs validation into `build-assets` or `create-release`
  as a hard release gate, or remove `continue-on-error` from the validation
  dependency before publishing assets.
- [ ] Validate every `GUEST_BINARIES` entry from `src/capsem/builder/docker.py`,
  including `capsem-dns-proxy` and `capsem-sysutil`.
- [ ] Derive the validation list from a single canonical source so future guest
  binaries cannot be missed by CI.
- [ ] Fail release if doctor scripts, benchmark scripts, or required guest
  artifacts are absent from the rootfs.

### T1.6 Documentation and Comments

- [ ] Update stale asset layout docs/comments to describe v2 manifests and
  hash-named installed assets.
- [ ] Update release-process notes if rootfs validation moves jobs.

## Proof Matrix

| Category | Required proof |
|---|---|
| Unit/contract | manifest producer tests cover same-day version increments and metadata preservation. |
| Functional | local `_pack-initrd` keeps both arch maps and valid `B3SUMS`. |
| Adversarial | stale hash files and legacy dirs are removed without touching live assets. |
| CI | rootfs validation failure blocks `create-release`. |
| Docs | asset layout docs describe v2 manifests and hash-named installed assets. |

## Verification

- [ ] `uv run pytest tests/test_docker.py::TestGenerateChecksums tests/test_gen_manifest.py tests/capsem-build-chain/test_manifest_regen.py -q`
- [ ] `uv run pytest tests/test_release_workflow.py::test_create_release_preserves_binary_metadata -q`
- [ ] `uv run pytest tests/capsem-build-chain/test_create_hash_assets.py tests/capsem-install/test_asset_download.py tests/capsem-install/test_installed_layout.py -q`
- [ ] `cargo test -p capsem-core asset_manager -- --nocapture`
- [ ] Release workflow dry-run or CI assertion proves rootfs validation failure
  blocks `create-release`.

## Exit Criteria

- [ ] No manifest mutation drops `min_assets`.
- [ ] Full build and initrd repack choose the same next asset version.
- [ ] Rootfs validation covers every canonical guest binary.
- [ ] Release workflow cannot publish assets after a rootfs validation failure.
