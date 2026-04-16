# Sprint: Orthogonal CI -- Separate Binary and Asset Pipelines

## What

Split the single release pipeline into two independent CI workflows: one for binaries (Rust + frontend), one for VM assets (kernel, rootfs, initrd). Each publishes independently and updates its own section of the v2 manifest. Binary releases no longer rebuild assets; asset releases no longer rebuild binaries.

## Why

- Binary version is `1.0.{timestamp}`, changing on every build. Assets change rarely (kernel updates, rootfs rebuilds).
- Current `cut-release` rebuilds everything, coupling binary and asset lifecycles. A typo fix in the CLI triggers a 450MB rootfs rebuild.
- The v2 manifest (from the asset refactor sprint) already has separate `assets` and `binaries` sections with compatibility ranges (`min_binary`, `min_assets`). The CI just needs to produce them independently.
- Faster releases: binary-only releases skip the 10+ minute image build. Asset-only releases skip the full test suite for Rust code.

## Prerequisites (all done)

- [x] v2 manifest format (`ManifestV2` in `asset_manager.rs`)
- [x] Hash-based asset filenames via hardlinks (`scripts/create_hash_assets.py`)
- [x] `min_binary` / `min_assets` compatibility ranges in manifest
- [x] `gen_manifest.py` and `generate_checksums()` produce v2 format
- [x] Service resolves assets via `ManifestV2::resolve()` (arch subdir + flat fallback)
- [x] `capsem-process` accepts `--kernel`/`--initrd`/`--rootfs` individual paths
- [x] `capsem status` shows asset version and per-file health
- [x] `_pack-initrd` creates hash-named hardlinks and skips docker when binaries are current
- [x] CI `release.yaml` accumulates v2 manifests
- [x] `just install` depends on `_check-assets` + `_pack-initrd`

## Key Decisions

- Two GitHub Actions workflows: `release-binary.yaml` and `release-assets.yaml`
- Shared manifest.json in GitHub Releases -- each workflow updates only its section
- `just cut-release` becomes binary-only. New `just cut-asset-release` for assets.
- Asset version scheme: `YYYY.MMDD.patch` (e.g., `2026.0415.1`)
- Binary version scheme: `1.0.{timestamp}` (already in place)

## Deliverables

### 1. Binary CI (`release-binary.yaml`)

- [ ] Trigger: push to `main` with tag `v1.0.*` (or manual dispatch)
- [ ] Build: `cargo build --release` for host crates, `cargo tauri build`, frontend
- [ ] Test: `just test` (unit + integration + cross-compile)
- [ ] Publish: DMG, deb, Tauri updater artifacts
- [ ] Manifest update: add entry to `binaries.releases`, set `binaries.current`, set `min_assets` to the current asset version
- [ ] Does NOT build kernel, rootfs, or initrd
- [ ] Does NOT upload asset files

### 2. Asset CI (`release-assets.yaml`)

- [ ] Trigger: push to `main` with tag `assets-YYYY.MMDD.*` (or manual dispatch, or changes to `guest/config/`, `guest/artifacts/`, kernel defconfig)
- [ ] Build: `capsem-builder build` for kernel + rootfs per arch, `_pack-initrd` for initrd
- [ ] Hash: compute blake3 hashes, generate hash-based filenames
- [ ] Publish: upload per-arch assets (`{arch}-vmlinuz`, `{arch}-initrd.img`, `{arch}-rootfs.squashfs`) to GitHub Releases
- [ ] Manifest update: add entry to `assets.releases`, set `assets.current`, set `min_binary` to the oldest compatible binary version
- [ ] Does NOT build Rust binaries or frontend

### 3. Manifest accumulation

- [ ] Each workflow downloads the current manifest from the latest release
- [ ] Merges its section (binary or asset) into the existing manifest
- [ ] Preserves the other section untouched
- [ ] Signs the merged manifest with minisign
- [ ] Uploads as release artifact

### 4. `just cut-release` (binary-only)

- [ ] Stamps `1.0.{timestamp}` version (already done)
- [ ] Runs `just test`
- [ ] Sets `min_assets` to the current asset version from `assets/manifest.json`
- [ ] Commits, tags `v{version}`, pushes
- [ ] Does NOT bump asset version

### 5. `just cut-asset-release`

- [ ] New recipe
- [ ] Stamps asset version as `YYYY.MMDD.{patch}` (auto-increment patch if date matches existing)
- [ ] Runs asset-specific tests (`just smoke` or capsem-doctor)
- [ ] Sets `min_binary` (default: `1.0.0` unless a breaking change)
- [ ] Commits, tags `assets-{version}`, pushes
- [ ] Does NOT bump binary version

### 6. `capsem update` -- independent updates

Currently stubbed (see `crates/capsem/src/update.rs:159`). Needs:

- [ ] Fetch v2 manifest from GitHub Releases
- [ ] Check for binary updates and asset updates separately
- [ ] Binary-only update: download new binary, keep existing assets
- [ ] Asset-only update: download hash-named asset files to `~/.capsem/assets/{arch}/`
- [ ] Both: download both
- [ ] Compatibility check: new binary's `min_assets` <= installed asset version
- [ ] Warn user if their assets are deprecated or incompatible with the new binary
- [ ] `cleanup_unused_assets()` after successful download

### 7. Installer and first-launch setup

- [ ] `capsem setup` downloads assets on first launch (setup.rs `step_welcome` is currently a stub -- see line 173)
- [ ] Download uses v2 manifest to determine which hash-named files to fetch
- [ ] Downloaded files go to `~/.capsem/assets/{arch}/{hash_filename}`
- [ ] Progress reporting during download (channel-based, like the old `BackgroundProgress`)
- [ ] `build-pkg.sh` bundles only `manifest.json` (already the case)
- [ ] Verify launchd unit `--assets-dir` resolves correctly for both symlink (dev) and real dir (installed)

### 8. Documentation site (`docs/`)

- [ ] Release pages distinguish binary releases from asset releases
- [ ] Document the v2 manifest format
- [ ] Document the asset/binary version independence
- [ ] Update "getting started" to explain that assets download on first launch
- [ ] Add a page for the asset versioning scheme

### 9. Release skill (`skills/release-process/SKILL.md`)

- [ ] Document `just cut-release` as binary-only
- [ ] Document `just cut-asset-release` as new recipe
- [ ] Document the two CI workflows and when to use each
- [ ] Document manifest accumulation and signing
- [ ] Document compatibility ranges and deprecation
- [ ] Update the pre-release checklist for both types

### 10. Asset pipeline skill (`skills/asset-pipeline/SKILL.md`)

- [ ] Document hash-based flat layout
- [ ] Document asset version scheme (`YYYY.MMDD.patch`)
- [ ] Document `cut-asset-release` workflow
- [ ] Document when assets need a new release vs when they don't
- [ ] Document compatibility with binary versions

### 11. Multi-arch manifest and hash assets

Local dev (`_pack-initrd`) only builds and hashes the host arch. CI must:

- [ ] Build assets for both arm64 and x86_64
- [ ] `gen_manifest.py` / `generate_checksums()` produces manifest with both arches
- [ ] `create_hash_assets.py` creates hash-named files for both arches
- [ ] `just cross-compile` (if it touches assets) regenerates manifest for all arches
- [ ] Verify `ManifestV2::resolve()` works for both arches from a single manifest

### 12. Asset upload format

CI currently uploads assets with arch-prefixed names (`arm64-rootfs.squashfs`). With hash-based filenames, the upload naming needs to match what `capsem update` downloads:

- [ ] CI uploads hash-named files: `arm64-rootfs-{hash16}.squashfs` (or keep arch prefix separate)
- [ ] `capsem update` download URL construction matches the upload naming
- [ ] Manifest `release_url()` still works (currently `https://github.com/google/capsem/releases/download/v{version}`)
- [ ] Decide: one GitHub Release per asset version, or assets attached to binary releases?

## Non-Goals

- Automatic detection of "did assets change?" to auto-trigger asset CI (future improvement -- for now, manual tag or explicit path trigger)
- Binary/asset matrix testing (testing every binary version against every asset version) -- compatibility ranges are sufficient
- Multi-channel releases (stable/beta/nightly) -- single channel for now
