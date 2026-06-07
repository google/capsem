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
- `tests/test_release_workflow_policy.py`
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

## Swarm Transfer Tracker

| Source | Priority | Owner task | Required transfer point | Required proof |
|---|---:|---|---|---|
| FD04 core-policy-assets | P1 | T1.1, T1.2 | Asset version selection compared version strings lexicographically, so same-day `.10` could sort before `.2`. | Done: resolver/merge tests cover `.2`, `.9`, `.10` and select numeric/latest order. |
| FD04 core-policy-assets | P2 | T1.4 | Asset cleanup skipped arch subdirectories and legacy dirs. | Done: arch-subdir stale hash fixture and `v1.0.*` legacy dir fixture pass without deleting live assets. |
| FD09 guest-image-builder | P1 | T1.3, T1.5 | Release-built initrds could miss `capsem-sysutil`; rootfs validation was stale and not hard-gated. | Done: rootfs validation is in `build-assets`, `create-release` directly depends on it, and initrd/security tests require every canonical guest binary. Runtime `/run` sysutil symlink proof remains in diagnostics/T10 VM gates. |
| FD09 guest-image-builder | P1 | T1.3 | `_pack-initrd` could rewrite the manifest as host-arch-only. | Done: `_pack-initrd` recomputes `B3SUMS` for all complete arch dirs; manifest-regeneration test pins the recipe. |
| FD09 guest-image-builder | P1 | T1.2 | Full-build and repack manifest versioning disagreed on same-day patches. | Done: `generate_checksums()` and `gen_manifest.py` share `manifest_version.next_asset_version`; tests cover same-day collision behavior. |
| FD09 guest-image-builder | P2 | T1.4 | Asset cleanup did not match installed per-arch layout. | Done: per-arch stale hash and legacy directory fixtures pass. |
| FD09 guest-image-builder | P2 | T1.3, T1.5 | Initrd/security tests used stale three-binary contracts and skipped missing files. | Done: tests import canonical `GUEST_BINARIES`, including `capsem-dns-proxy` and `capsem-sysutil`, and fail on absence. |
| FD09 guest-image-builder | P2 | T1.5 | Guest artifact permissions are inconsistent for `capsem-doctor`. | Done: rootfs validator covers doctor/bench/scripts; live in-VM permission proof remains in T10/T11. |
| FD10 ci-packaging | P1 | T1.5 | Rootfs validation missed required guest binaries and was not a hard gate. | Done: CI derives validation from canonical `GUEST_BINARIES`/rootfs artifact constants and blocks `create-release` through direct `build-assets` dependency. |
| FD10 ci-packaging | P1 | T1.1 | Release manifest binary metadata was overwritten and no regression test existed. | Done: `tests/test_release_workflow_policy.py::test_create_release_preserves_binary_metadata` executes the embedded release script fixture. |
| FD13 ci-release-landing-1-1 | P1 | T1.5 | Rootfs validation in release workflow was narrower than sprint checklist. | Done: local preflight and CI use the same canonical rootfs validator. |

## Task List

### T1.1 Preserve Binary Compatibility Metadata

- [x] Preserve existing `min_assets`, `date`, and `deprecated` fields when
  adding binary file metadata in release manifest population.
- [x] Add a regression fixture where create-release updates binary file
  metadata without changing the selected asset release for older binaries.
- [x] Assert `ManifestV2::pick_asset_version` sees the expected `min_assets`
  after release manifest mutation.

### T1.2 Unify Asset Version Generation

- [x] Make `generate_checksums()` share the same same-day patch increment
  behavior as `scripts/gen_manifest.py`.
- [x] Move same-day patch selection into a shared helper or duplicate it with
  tests comparing both producers against an existing same-day manifest.
- [x] Add a collision test for two same-day full asset builds.

### T1.3 Safe Local Initrd Repack

- [x] Change `_pack-initrd` manifest regeneration so it updates only the host
  arch entry while preserving other arch maps, or recomputes all arch `B3SUMS`
  entries when both arch dirs exist.
- [x] Add/adjust tests that assert two-arch manifests survive local repacks.
- [x] Ensure local repack does not silently drop x86_64 maps on arm64 hosts or
  arm64 maps on x86_64 hosts.

### T1.4 Asset Cleanup

- [x] Teach `cleanup_unused_assets()` to traverse arch subdirectories.
- [x] Remove legacy `v1.0.*` directories.
- [x] Add adversarial cleanup fixtures that include live hash files, stale hash
  files, arch subdirs, and legacy dirs.
- [x] Assert cleanup never deletes a referenced non-deprecated asset.

### T1.5 Rootfs Validation as a Hard Gate

- [x] Move squashfs/rootfs validation into `build-assets` or `create-release`
  as a hard release gate, or remove `continue-on-error` from the validation
  dependency before publishing assets.
- [x] Validate every `GUEST_BINARIES` entry from `src/capsem/builder/docker.py`,
  including `capsem-dns-proxy` and `capsem-sysutil`.
- [x] Derive the validation list from a single canonical source so future guest
  binaries cannot be missed by CI.
- [x] Fail release if doctor scripts, benchmark scripts, or required guest
  artifacts are absent from the rootfs.

### T1.6 Documentation and Comments

- [x] Update stale asset layout docs/comments to describe v2 manifests and
  hash-named installed assets.
- [x] Update release-process notes if rootfs validation moves jobs.

## Proof Matrix

| Category | Required proof |
|---|---|
| Unit/contract | manifest producer tests cover same-day version increments and metadata preservation. |
| Functional | local `_pack-initrd` keeps both arch maps and valid `B3SUMS`. |
| Adversarial | stale hash files and legacy dirs are removed without touching live assets. |
| CI | rootfs validation failure blocks `create-release`. |
| Docs | asset layout docs describe v2 manifests and hash-named installed assets. |

## Verification

- [x] `env UV_CACHE_DIR=/private/tmp/capsem-uv-cache uv run pytest tests/test_docker.py::TestGenerateChecksums tests/test_gen_manifest.py tests/capsem-build-chain/test_manifest_regen.py tests/capsem-build-chain/test_create_hash_assets.py tests/test_release_workflow_policy.py tests/test_validate.py::TestE302 tests/capsem-rootfs-artifacts/test_rootfs_artifacts.py -q` (45 passed; rerun of `tests/test_release_workflow_policy.py -q` after heredoc fixture: 8 passed).
- [x] `cargo test -p capsem-core asset_manager -- --nocapture` (43 passed).
- [x] `cargo test -p capsem paths -- --nocapture` (15 passed).
- [x] `bash -n scripts/validate-rootfs.sh scripts/check-release-workflow.sh scripts/doctor-common.sh scripts/gen_manifest.py`.
- [x] `env PYTHONPYCACHEPREFIX=/private/tmp/capsem-pycache python3 -m py_compile src/capsem/builder/manifest_version.py scripts/gen_manifest.py src/capsem/builder/docker.py src/capsem/builder/doctor.py src/capsem/builder/validate.py`.
- [x] Release workflow static CI assertion proves rootfs validation failure
  blocks `create-release`.

## Exit Criteria

- [x] No manifest mutation drops `min_assets`.
- [x] Full build and initrd repack choose the same next asset version.
- [x] Rootfs validation covers every canonical guest binary.
- [x] Release workflow cannot publish assets after a rootfs validation failure.
