---
name: release-process
description: Capsem release process, CI pipeline, Apple code signing, notarization, documentation site, and post-release verification. Use when preparing a release, debugging CI failures, working with Apple certificates, updating the documentation site, or cutting a new version. Covers the full release lifecycle from pre-release checklist through post-release verification.
---

# Release Process

## Pre-release checklist

```bash
just doctor                    # Check tools
just build-assets              # Rebuild VM assets if needed
scripts/preflight.sh           # Validate Apple certs for CI
just full-test                 # Unit + doctor + integration + bench
```

## Cutting a release

### Automated (preferred)

```bash
just cut-release
```

Bumps patch version, stamps changelog, commits, tags, pushes, waits for CI.

### Manual

1. Bump version in both `Cargo.toml` (workspace) and `crates/capsem-app/tauri.conf.json`
2. Move `[Unreleased]` changelog items into `[X.Y.Z] - YYYY-MM-DD`
3. Create/update release page at `site/src/content/docs/releases/<major>-<minor>.md`
4. `scripts/preflight.sh` then `just full-test`
5. Commit, tag `vX.Y.Z`, push both

Never reuse or move a tag. Always increment the version number.

## CI pipeline (release.yaml)

Triggered by `vX.Y.Z` tag push. Sequential jobs:

| Job | Runner | Purpose |
|-----|--------|---------|
| `preflight` | macos-14 | Fail-fast: Apple cert import, Tauri key |
| `build-assets` (arm64 + x86_64) | ubuntu-24.04-arm, ubuntu-24.04 | Kernel + rootfs via Docker (parallel matrix) |
| `test` | macos-14 | Unit tests, cross-compile, frontend build |
| `build-app-macos` | macos-14 | Tauri build, codesign, notarize, DMG |
| `build-app-linux` (arm64 + x86_64) | ubuntu-24.04-arm, ubuntu-24.04 | Tauri build, deb + AppImage (parallel matrix) |
| `create-release` | ubuntu-latest | GitHub release with all artifacts |

### CI invariants (hard-won lessons)

- **Use justfile recipes in CI.** `build-assets` must call `just build-kernel` and `just build-rootfs`, not reimplement the builder commands. Drift between the justfile and CI caused v0.14.2-v0.14.4 to ship without vmlinuz/initrd.img.
- **Build both kernel and rootfs.** The builder defaults to `--template rootfs` only. The kernel template must be built explicitly.
- **`assets/current` must be a real directory, not a symlink.** `generate_checksums()` creates a symlink, but GitHub Actions strips symlinks from artifacts. After calling `generate_checksums`, replace the symlink with `rm -rf assets/current && cp -r assets/arm64 assets/current`.
- **`Cargo.lock` is gitignored.** CI resolves a fresh lockfile each build. This means dependency versions can drift between builds. Acceptable for now but a reproducibility risk.
- **Verify assets before Tauri build.** The `Verify assets layout` step lists assets/arm64/ and assets/current/ to catch missing files early. Tauri's build.rs resolves `../../assets/current/vmlinuz` relative to `crates/capsem-app/`.
- **`pyproject.toml` version must match.** Three files must be bumped in sync: `Cargo.toml` (workspace), `crates/capsem-app/tauri.conf.json`, `pyproject.toml`. `just cut-release` handles this automatically.

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
