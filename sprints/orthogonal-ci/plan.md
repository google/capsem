# Sprint: Orthogonal CI -- Separate Binary and Asset Pipelines

## What

Split the single release pipeline into two independent CI workflows: one for binaries (Rust + frontend), one for VM assets (kernel, rootfs, initrd). Each publishes independently and updates its own section of the v2 manifest. Binary releases no longer rebuild assets; asset releases no longer rebuild binaries.

## Why

- Binary version is `1.0.{timestamp}`, changing on every build. Assets change rarely (kernel updates, rootfs rebuilds).
- Current `cut-release` rebuilds everything, coupling binary and asset lifecycles. A typo fix in the CLI triggers a 450MB rootfs rebuild.
- The v2 manifest (from the asset refactor sprint) already has separate `assets` and `binaries` sections with compatibility ranges (`min_binary`, `min_assets`). The CI just needs to produce them independently.
- Faster releases: binary-only releases skip the 10+ minute image build. Asset-only releases skip the full test suite for Rust code.

## Prerequisites

- v2 manifest format landed (asset refactor sprint)
- Hash-based flat asset layout deployed
- `min_binary` / `min_assets` compatibility ranges working

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

- [ ] Check for binary updates and asset updates separately
- [ ] Binary-only update: download new binary, keep existing assets
- [ ] Asset-only update: download new asset files (hash-named), keep existing binary
- [ ] Both: download both
- [ ] Compatibility check: new binary's `min_assets` <= installed asset version, and vice versa
- [ ] Warn user if their assets are deprecated or incompatible with the new binary

### 7. Installer (`service_install.rs`, `build-pkg.sh`)

- [ ] Installer no longer bundles assets in the DMG/pkg
- [ ] First-launch setup downloads assets from GitHub Releases (setup.rs `step_welcome` is currently a stub)
- [ ] `capsem update` asset download rewritten for v2 manifest + hash-based filenames (update.rs `run_update` uses stubs)
- [ ] Installed service uses flat hash-based asset directory
- [ ] Verify `--assets-dir` in launchd/systemd unit points to `~/.capsem/assets/` (flat)

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

## Non-Goals

- Automatic detection of "did assets change?" to auto-trigger asset CI (future improvement -- for now, manual tag or explicit path trigger)
- Binary/asset matrix testing (testing every binary version against every asset version) -- compatibility ranges are sufficient
- Multi-channel releases (stable/beta/nightly) -- single channel for now
