---
name: release-process
description: Capsem release process, CI pipeline, Apple code signing, notarization, documentation site, and post-release verification. Use when preparing a release, debugging CI failures, working with Apple certificates, updating the documentation site, or cutting a new version. Covers the full release lifecycle from pre-release checklist through post-release verification.
---

# Release Process

## Pre-release checklist

```bash
just doctor                    # Check tools
scripts/preflight.sh           # Validate Apple certs for CI
just test                      # ALL tests: unit + integration + cross-compile + bench
```

## Cutting a release

### Automated (preferred)

```bash
just cut-release
```

Runs `test` (all tests including integration, cross-compile, benchmarks), then bumps patch version, stamps changelog, commits, tags, pushes, waits for CI.

### Manual

1. Bump version in both `Cargo.toml` (workspace) and `crates/capsem-app/tauri.conf.json`
2. Move `[Unreleased]` changelog items into `[X.Y.Z] - YYYY-MM-DD`
3. Create/update release page at `site/src/content/docs/releases/<major>-<minor>.md`
4. `scripts/preflight.sh` then `just test`
5. Commit, tag `vX.Y.Z`, push both

Never reuse or move a tag. Always increment the version number.

## CI pipeline (release.yaml)

Triggered by `vX.Y.Z` tag push. Parallelized pipeline (~18 min wall clock):

```
preflight (30s) ──> build-assets (arm64 + x86_64, 10 min) ──> build-app-macos (15 min) ──┐
                └──> test (8 min) ─────────────────────────────────────────────────────────├──> create-release
                └──────────────────> build-app-linux (arm64 + x86_64, 10 min) ────────────┘
```

| Job | Runner | Needs | Purpose |
|-----|--------|-------|---------|
| `preflight` | macos-14 | -- | Fail-fast: Apple cert, Tauri key, notarization |
| `build-assets` | ubuntu arm64 + x86_64 | preflight | Kernel + rootfs via Docker |
| `test` | macos-14 | preflight | Unit tests + coverage, frontend, audit |
| `build-app-macos` | macos-14 | preflight, build-assets | Tauri build, codesign, notarize, DMG |
| `build-app-linux` | ubuntu arm64 + x86_64 | preflight, build-assets | Tauri build, deb (+ AppImage on x86_64) |
| `create-release` | ubuntu-latest | test, build-app-macos, build-app-linux | Merge latest.json, sign manifest, GitHub release |

Test runs in parallel with builds. A test failure blocks `create-release` but doesn't delay compilation.

### CI invariants (hard-won lessons)

- **Per-arch VM assets use arch-prefixed names on GitHub.** CI uploads with `gh release upload "$f#${arch}-${base}"`, renaming `vmlinuz` to `arm64-vmlinuz`, `rootfs.squashfs` to `arm64-rootfs.squashfs`, etc. The manifest keeps bare filenames in its per-arch structure; `AssetManager.arch_prefix` handles the URL translation at download time. If you change the upload naming, update `AssetManager.download_url()` in `asset_manager.rs`.
- **Use justfile recipes in CI.** `build-assets` must call `just build-kernel` and `just build-rootfs`, not reimplement the builder commands. Drift between the justfile and CI caused v0.14.2-v0.14.4 to ship without vmlinuz/initrd.img.
- **Build both kernel and rootfs.** The builder defaults to `--template rootfs` only. The kernel template must be built explicitly.
- **`assets/current` must be a real directory, not a symlink.** `generate_checksums()` creates a symlink, but GitHub Actions strips symlinks from artifacts. After calling `generate_checksums`, replace the symlink with `rm -rf assets/current && cp -r assets/arm64 assets/current`.
- **`Cargo.lock` is gitignored.** CI resolves a fresh lockfile each build. This means dependency versions can drift between builds. Acceptable for now but a reproducibility risk.
- **Verify assets before Tauri build.** The `Verify assets layout` step lists assets/arm64/ and assets/current/ to catch missing files early. Tauri's build.rs resolves `../../assets/current/vmlinuz` relative to `crates/capsem-app/`.
- **`pyproject.toml` version must match.** Three files must be bumped in sync: `Cargo.toml` (workspace), `crates/capsem-app/tauri.conf.json`, `pyproject.toml`. `just cut-release` handles this automatically.
- **No AppImage on any platform.** linuxdeploy cannot run on GitHub CI runners -- Ubuntu 24.04 lacks FUSE2, and neither `libfuse2` nor `APPIMAGE_EXTRACT_AND_RUN=1` fixes it reliably. All Linux platforms ship `.deb` only. CI matrix passes `bundles: deb` for both arm64 and x86_64. `just cross-compile` matches this. This cost 14 consecutive failed releases (v0.12.1 through v0.14.14) to discover.
- **Tauri signing keys on all platforms.** `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` must be passed to every `cargo tauri build` step (macOS and Linux). Missing keys cause "public key found but no private key" failure. The macOS job had them from the start; the Linux job was missing them until v0.14.11.
- **Collect all updater artifacts.** Linux artifact collection must include `.tar.gz`, `.tar.gz.sig`, `.AppImage.tar.gz`, `.AppImage.tar.gz.sig` -- not just `.deb` and `.AppImage`. Tauri's updater needs the `.sig` files.
- **`just cross-compile` is not a perfect CI replica.** It runs in a docker container on macOS, which has FUSE (via Colima's Linux VM). CI runners may not have FUSE, so AppImage bundling that works locally can fail in CI. The recipe catches compile errors and most packaging issues, but environment differences (FUSE, linuxdeploy availability) can still slip through. Always verify the first CI run of a new Linux packaging change.
- **Platform-gate all macOS-only APIs.** Every use of `libc::clonefile`, `AppleVzHypervisor`, `core_foundation_sys`, etc. must be wrapped in `#[cfg(target_os = "macos")]` -- struct, impl, AND tests. The Linux app build compiles the full workspace. `cargo test --test platform_gating` catches ungated symbols at unit test time. This burned v0.14.7 through v0.14.9.
- **Pin Xcode version on macOS runners.** Always `sudo xcode-select -s /Applications/Xcode_16.2.app` (or latest) before any Apple toolchain use. GitHub periodically updates runner images and the default Xcode can break (Abort trap in xcodebuild). The preflight may pass on one runner instance while build-app-macos gets a different one. v0.14.12 failed because Xcode 15.4's xcodebuild crashed with `Abort trap: 6` when Tauri tried to locate notarytool -- despite zero workflow changes from v0.14.11 which passed 9 hours earlier.
- **`latest.json` is optional in `gh release create`.** Tauri only generates updater `latest.json` for bundle types that produce `.tar.gz` + `.sig` artifacts (AppImage, not deb). With deb-only builds, no `latest.json` exists. The create-release step must handle this gracefully.
- **AppImage was dropped after 14 failed releases.** linuxdeploy (a FUSE2 AppImage) cannot run on Ubuntu 24.04 CI runners (FUSE3 only). Tested: `libfuse2` install, `APPIMAGE_EXTRACT_AND_RUN=1` env var, both together -- none worked reliably. If AppImage support is needed in the future, the approach would be to pre-extract linuxdeploy (`--appimage-extract`) and run the extracted binary directly, bypassing FUSE entirely.

## Full-test gates

| Gate | What |
|------|------|
| Unit tests | `cargo llvm-cov` with coverage |
| Cross-compile | capsem-agent for aarch64 + x86_64 musl |
| Frontend | `pnpm run check && pnpm run build` |
| capsem-doctor | Boot VM, run full diagnostic suite |
| Integration | Boot VM, exercise all 6 telemetry pipelines |
| Benchmark | Boot VM, run capsem-bench |

## Apple code signing

### p12 encryption (critical gotcha)

macOS Keychain only accepts legacy PKCS12 (3DES/SHA1). OpenSSL 3.x creates PBES2/AES-256-CBC by default, which Keychain rejects with "wrong password."

Check: `openssl pkcs12 -in cert.p12 -info -nokeys -nocerts -passin pass:PWD 2>&1 | head -5`
- `PBES2` = broken on macOS
- `pbeWithSHA1And3-KeyTripleDES-CBC` = works

Fix: `scripts/fix_p12_legacy.sh` then `gh secret set APPLE_CERTIFICATE < private/apple-certificate/capsem-b64.txt`

### Notarization

CI uses `--skip-stapling` (async). First-time can take hours. Verify locally:
```bash
xcrun notarytool history --key private/apple-certificate/capsem.p8 --key-id KEY_ID --issuer ISSUER_ID
```

## CI secrets

| Secret | Purpose |
|--------|---------|
| `APPLE_CERTIFICATE` | Base64 `.p12` (legacy 3DES) |
| `APPLE_CERTIFICATE_PASSWORD` | Password for p12 |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: Elie Bursztein (L8EGK4X86T)` |
| `APPLE_API_ISSUER` | App Store Connect issuer UUID |
| `APPLE_API_KEY` | App Store Connect key ID |
| `APPLE_API_KEY_PATH` | Contents of `.p8` private key |
| `TAURI_SIGNING_PRIVATE_KEY` | Tauri updater minisign key |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Password for Tauri key |
| `CODECOV_TOKEN` | Codecov upload token |

Local backups: `private/apple-certificate/` and `private/tauri/` (gitignored).

## Post-release verification

```bash
gh release view vX.Y.Z
gh release download vX.Y.Z --pattern manifest.json -D /tmp/verify
gh release download vX.Y.Z --pattern '*.dmg' -D /tmp/verify
hdiutil attach /tmp/verify/Capsem*.dmg -nobrowse -readonly
```

## Documentation site

The product website uses Astro Starlight. Docs live in `site/src/content/docs/`.

### Writing style
Tight and to the point. One topic per page. Tables over prose for configs and test cases. No filler.

### Structure
- `site/src/content/docs/<category>/<topic>.md`
- Categories: `security/`, `testing/`, `releases/`, `architecture/`
- Frontmatter: `title` and `description` required. `sidebar: { order: N }` for ordering.

### Release pages
- Path: `site/src/content/docs/releases/<major>-<minor>.md` (hyphens, not dots)
- Each page consolidates all patch releases for that minor
- Higher `sidebar.order` = newer = listed first

### Dev workflow
```bash
cd site && pnpm run dev     # localhost:4321
cd site && pnpm run build   # Production build
```

### Keep docs in sync
When features change (settings, CLI flags, MCP tools, security invariants, benchmarks), update the corresponding doc page. When cutting a new minor, create a new release page.

## Changelog

Keep a Changelog format in `CHANGELOG.md`. Every user-visible change gets an entry under `## [Unreleased]` using: Added, Changed, Deprecated, Removed, Fixed, Security.

## Versioning

SemVer. Single source of truth: `workspace.package.version` in root `Cargo.toml`. Keep `crates/capsem-app/tauri.conf.json` in sync manually.

## Commits

1. Include `CHANGELOG.md` update in the same commit
2. Stage files explicitly (no `git add -A`)
3. Conventional messages: `feat:`, `fix:`, `chore:`, `docs:`
4. Author: Elie Bursztein <github@elie.net>
5. No `Co-Authored-By` trailers
