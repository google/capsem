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

`minisign` is a first-class local release prerequisite. `bootstrap.sh`,
`just doctor`, `just doctor fix`, and `scripts/preflight.sh` must all surface it
before any local install, `just exec`, asset sync, or package signing path can
claim to be healthy.

Release asset manifests are generated through `capsem-admin manifest generate`.
Do not publish or document alternate manifest writers.

## Cutting a release

### Release history discipline

Release history is forward-only. Once a commit or tag has been pushed, do not
amend it, force-push it, or force-move the tag to "save" that release. That
makes the release harder to audit and can leave CI, GitHub Releases, and local
checkouts disagreeing about what was actually shipped.

- Never use `git commit --amend`, `git push --force`, `git push --force-with-lease`,
  `git tag -f`, or a forced tag push for a release that has already left the
  machine.
- If a pushed release commit or tag fails CI, land a normal follow-up commit on
  top of `main`, stamp a new unique version, create a new tag, and push forward.
- Cancel superseded failed CI runs when useful, but leave the historical commit
  and tag alone. The goal is a clean next release, not rewriting the failed one.
- Do not reuse a version string or tag name. For the `1.2.{unix_timestamp}`
  release line, choose a later timestamp and let the old tag remain historical.

### Prepare release commit and local tag

```bash
just cut-release
```

Runs `test` (all tests including integration, cross-compile, benchmarks), then
bumps the version, stamps the changelog, creates the release commit, and creates
a local `vX.Y.Z` tag. It does **not** push. Push the branch and tag manually
after checking the local commit/tag.

### Manual publish

1. Confirm the release tag does not already exist remotely:
   `git ls-remote origin "refs/tags/vX.Y.Z"`
2. Push the release commit to `main`: `git push origin HEAD:main`
3. Push the immutable tag: `git push origin vX.Y.Z`
4. Watch the tag workflow: `just release vX.Y.Z`

Never reuse or move a tag. Always increment the version number, and always tag
forward.

### GitHub CLI release control

Use `gh` as the release control plane:

```bash
gh auth status
gh release list --limit 10
git ls-remote origin "refs/tags/vX.Y.Z"
git push origin HEAD:main
git push origin vX.Y.Z
gh run watch <run-id>
gh run view <run-id> --json status,conclusion,headSha,url
gh run view <run-id> --log-failed
gh release view vX.Y.Z --json name,tagName,isDraft,isPrerelease,assets,url
```

Before pushing a tag, confirm the tag does not already exist remotely. After
pushing, watch the release workflow to completion. If CI fails, use
`gh run view --log-failed` to diagnose, make a forward fix, and cut the next tag.

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
| `build-app-macos` | macos-14 | preflight, build-assets | Tauri `.app` build, companion binaries, `scripts/build-pkg.sh`, notarize + staple `.pkg` |
| `build-app-linux` | ubuntu arm64 + x86_64 | preflight, build-assets | Tauri build, deb (+ AppImage on x86_64) |
| `create-release` | ubuntu-latest | test, build-app-macos, build-app-linux | Merge latest.json, sign manifest, GitHub release |

Test runs in parallel with builds. A test failure blocks `create-release` but doesn't delay compilation.

### CI invariants (hard-won lessons)

- **CI is a clean checkout.** If the build depends on a generated source file,
  either track it or regenerate it in CI before the consumer imports it. A local
  generated file hidden by `.gitignore` can pass local tests and fail immediately
  in GitHub Actions. The frontend `mock-settings.generated.ts` file is an example:
  `mock-settings.ts` imports it, so it must exist in a clean checkout or be
  generated by the workflow.
- **Install E2E needs real package assets in a clean checkout.** The release
  `test-install` job must download or build `assets/<arch>/`, regenerate and
  locally sign `assets/manifest.json`, and then repack the `.deb` with an
  absolute assets directory plus explicit output path. If `assets/` is missing
  and `scripts/repack-deb.sh` receives the bare word `assets`, it can otherwise
  be mistaken for an output file and leave the original Tauri `.deb` unrepacked.
- **Clean-checkout proof belongs before tagging.** When fixing release-only
  failures, test the exact path a runner takes: fresh checkout, install deps,
  then focused checks (`pnpm -C frontend run check`, generated-config conformance
  tests, `pnpm -C frontend run test`, `pnpm -C frontend run build`) before the
  full release gate.
- **Per-arch VM assets use arch-prefixed names on GitHub.** CI uploads with `gh release upload "$f#${arch}-${base}"`, renaming `vmlinuz` to `arm64-vmlinuz`, etc. The v2 manifest keeps bare filenames in per-arch `arches` maps.
- **Use justfile recipes in CI.** `build-assets` must call `just build-kernel` and `just build-rootfs`, not reimplement the builder commands. Drift between the justfile and CI caused v0.14.2-v0.14.4 to ship without vmlinuz/initrd.img.
- **Build both kernel and rootfs.** The builder defaults to `--template rootfs` only. The kernel template must be built explicitly.
- **`assets/current` must be a real directory, not a symlink.** `generate_checksums()` creates a symlink, but GitHub Actions strips symlinks from artifacts. After calling `generate_checksums`, replace the symlink with `rm -rf assets/current && cp -r assets/arm64 assets/current`.
- **`Cargo.lock` is gitignored.** CI resolves a fresh lockfile each build. This means dependency versions can drift between builds. Acceptable for now but a reproducibility risk.
- **Verify assets before Tauri build.** The `Verify assets layout` step lists assets/arm64/ and assets/current/ to catch missing files early. Tauri's build.rs resolves `../../assets/current/vmlinuz` relative to `crates/capsem-app/`.
- **Three files hold the binary version.** `Cargo.toml` (workspace), `crates/capsem-app/tauri.conf.json`, `pyproject.toml`. `just _stamp-version` handles all three automatically. `just cut-release` and `just install` both call it.
- **Install manifest-signing tools before signing.** Linux app release jobs must
  install `minisign` before the package payload manifest signing step. Installing
  it later with Tauri system dependencies is too late because
  `Sign package payload manifest` runs immediately after `Generate manifest`.
  The install E2E can still pass while the release Linux app jobs fail here, so
  keep a static workflow policy test for the step ordering.
- **Local manifest signing is part of setup, not a release afterthought.**
  `bootstrap.sh` must install `minisign` on macOS with Homebrew when available,
  `capsem-doctor` must list it under `Manifest Signing Tools`, and `just doctor
  fix` must auto-install it on macOS like the rest of the fixable toolchain.
  Local VM assets use a signed manifest too; if `just exec`, `just install`, or
  `scripts/sync-dev-assets.sh` signs `assets/manifest.json`, a machine without
  `minisign` is not actually ready.
- **Do not make macOS CI depend on a Homebrew-only `flock` binary.** GitHub's
  macOS runners do not provide `flock`, even when developer machines do.
  Shared `just` execution locking must work with the checked-in
  `scripts/lib/exec_lock.sh` fallback: use `flock` when it exists and a Python
  `fcntl.flock` holder process otherwise. Keep `flock` out of `capsem-doctor`
  required tools unless the fallback is removed.
- **Treat the PR Python schema lane as a scoped contract gate, not the full
  Python coverage gate.** The macOS PR job intentionally runs
  `tests/test_*.py` so it does not boot VM suites; on a clean GitHub macOS
  runner that top-level subset reports about 88.67% coverage, so the workflow
  floor is 89%. The complete local `just test` Python stage still runs the full
  suite and keeps its 90% floor.
- **Do not execute artifact-dependent Python suites on a clean PR runner before
  creating their artifacts.** `tests/capsem-bootstrap/` needs real
  `assets/<arch>/` plus `assets/manifest.json`, and `tests/capsem-codesign/`
  needs built, signed host binaries. The PR macOS no-VM integration lane runs
  only suites without generated prerequisites and then import-collects every
  `tests/capsem-*/` suite; the full `just test` gate owns bootstrap/codesign
  execution after `_pack-initrd`/`_sign` have made the prerequisites real.
- **Do not run live KVM probes on GitHub-hosted PR runners.** Hosted ARM runners
  can expose `/dev/kvm` but still hang or behave inconsistently under test
  execution. PR Linux CI sets `CAPSEM_SKIP_KVM_TESTS=1` and runs
  `cargo test --no-run --all-targets` for the portable host crates: it compiles
  the KVM backend and Linux test binaries without executing hosted-runner KVM
  probes, while release CI owns real-KVM exercise.
- **Ordinary CI must not hide red signals.** Diagnostic-only steps should not
  use `continue-on-error`; make the diagnostic command itself non-fatal so a
  green job does not carry a red annotation. Test steps must not end in
  `|| true`, coverage summary pipes must use `set -o pipefail`, and Codecov
  test analytics should use `codecov/codecov-action@v5` with
  `report_type: test_results`.
- **No AppImage on any platform.** linuxdeploy cannot run on GitHub CI runners -- Ubuntu 24.04 lacks FUSE2, and neither `libfuse2` nor `APPIMAGE_EXTRACT_AND_RUN=1` fixes it reliably. All Linux platforms ship `.deb` only. CI matrix passes `bundles: deb` for both arm64 and x86_64. `just cross-compile` matches this. This cost 14 consecutive failed releases (v0.12.1 through v0.14.14) to discover.
- **Tauri signing keys on all platforms.** `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` must be passed to every `cargo tauri build` step (macOS and Linux). Missing keys cause "public key found but no private key" failure. The macOS job had them from the start; the Linux job was missing them until v0.14.11.
- **Collect all updater artifacts.** Linux artifact collection must include `.tar.gz`, `.tar.gz.sig`, `.AppImage.tar.gz`, `.AppImage.tar.gz.sig` -- not just `.deb` and `.AppImage`. Tauri's updater needs the `.sig` files.
- **`just cross-compile` is not a perfect CI replica.** It runs in a docker container on macOS, which has FUSE (via Colima's Linux VM). CI runners may not have FUSE, so AppImage bundling that works locally can fail in CI. The recipe catches compile errors and most packaging issues, but environment differences (FUSE, linuxdeploy availability) can still slip through. Always verify the first CI run of a new Linux packaging change.
- **Platform-gate all macOS-only APIs.** Every use of `libc::clonefile`, `AppleVzHypervisor`, `core_foundation_sys`, etc. must be wrapped in `#[cfg(target_os = "macos")]` -- struct, impl, AND tests. The Linux app build compiles the full workspace. `cargo test --test platform_gating` catches ungated symbols at unit test time. This burned v0.14.7 through v0.14.9.
- **Pin Xcode version on macOS runners.** Always `sudo xcode-select -s /Applications/Xcode_16.2.app` (or latest) before any Apple toolchain use. GitHub periodically updates runner images and the default Xcode can break (Abort trap in xcodebuild). The preflight may pass on one runner instance while build-app-macos gets a different one. v0.14.12 failed because Xcode 15.4's xcodebuild crashed with `Abort trap: 6` when Tauri tried to locate notarytool -- despite zero workflow changes from v0.14.11 which passed 9 hours earlier.
- **Installer identity and Gatekeeper checks are release gates.** Release
  preflight must require `APPLE_INSTALLER_SIGNING_IDENTITY`, and it must start
  with `Developer ID Installer:`. Pass it into `scripts/build-pkg.sh` through
  the job environment, not inline expressions. After `xcrun stapler validate`,
  `build-app-macos` must run `pkgutil --check-signature` and
  `spctl -a -vv -t install` against the built `.pkg`. If a local macOS host
  reports Code Signing subsystem errors for multiple known-good releases, treat
  the host as suspect, but keep the CI macOS gate release-blocking.
- **Package metadata versions must match the release tag exactly.** The release
  validators compare `.deb` control metadata and `.pkg` distribution metadata
  to `GITHUB_REF_NAME#v`. Do not append a build timestamp in repackaging
  scripts; local install paths already stamp a fresh version before packaging
  when they need upgrade ordering. macOS `.pkg` manifest validation must also
  expand into a fresh directory or remove the previous expansion first.
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

Shipping artifact on macOS is a **`.pkg`** (productbuild), not a `.dmg`. Flow:

1. `cargo tauri build --bundles app --skip-stapling` -- builds `.app` only (Tauri skips stapling the inner app; we staple the outer `.pkg`).
2. `scripts/build-pkg.sh` -- productbuilds `Capsem-$VERSION.pkg` with the `.app` + companion binaries + `manifest.json`. Heavy VM assets are downloaded on first use by the postinstall.
3. `xcrun notarytool submit ... --wait --timeout 30m` -- synchronous.
4. `xcrun stapler staple` + `xcrun stapler validate`.

Verify credentials locally (before touching a tag):
```bash
xcrun notarytool history --key private/apple-certificate/capsem.p8 --key-id KEY_ID --issuer ISSUER_ID
```

**403 "A required agreement is missing or has expired"** -- Apple periodically refreshes the Developer Program License Agreement, Paid Apps Agreement, etc. Only the **Account Holder** (not Admin/Developer) can accept. Check banners at both:
- https://developer.apple.com/account (Program License Agreement)
- https://appstoreconnect.apple.com → Agreements, Tax, and Banking (Free/Paid Apps)

Propagation can lag 1-5 min after accepting. `notarytool history` must return a list (possibly empty) before you tag -- the CI preflight step runs the same check and fails fast on 403.

## CI secrets

| Secret | Purpose |
|--------|---------|
| `APPLE_CERTIFICATE` | Base64 `.p12` (legacy 3DES) |
| `APPLE_CERTIFICATE_PASSWORD` | Password for p12 |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: Elie Bursztein (L8EGK4X86T)` |
| `APPLE_INSTALLER_SIGNING_IDENTITY` | `Developer ID Installer: Elie Bursztein (L8EGK4X86T)` |
| `APPLE_API_ISSUER` | App Store Connect issuer UUID |
| `APPLE_API_KEY` | App Store Connect key ID |
| `APPLE_API_KEY_PATH` | Contents of `.p8` private key |
| `TAURI_SIGNING_PRIVATE_KEY` | Tauri updater minisign key |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Password for Tauri key |
| `CODECOV_TOKEN` | Codecov upload token |

CI secrets are the source of truth for release signing. Local backups in
`private/apple-certificate/` and `private/tauri/` are useful for local preflight
and packaging checks, but they are gitignored and must never be staged.

## Post-release verification

```bash
gh release view vX.Y.Z
gh release download vX.Y.Z --pattern manifest.json -D /tmp/verify
gh release download vX.Y.Z --pattern manifest.json.minisig -D /tmp/verify
minisign -Vm /tmp/verify/manifest.json -x /tmp/verify/manifest.json.minisig -p config/manifest-sign.pub
gh release download vX.Y.Z --pattern '*.pkg' -D /tmp/verify
pkgutil --check-signature /tmp/verify/Capsem-*.pkg
spctl -a -vv -t install /tmp/verify/Capsem-*.pkg      # Gatekeeper accepts notarized+stapled
xcrun stapler validate /tmp/verify/Capsem-*.pkg       # Staple ticket present
gh release download vX.Y.Z --pattern '*.deb' -D /tmp/verify
python3 scripts/verify_deb_payload.py /tmp/verify/*.deb --minisign-pubkey config/manifest-sign.pub
```

Use `scripts/verify_deb_payload.py` for `.deb` inspection instead of ad hoc
`tar`/`strings` checks. It validates control metadata, companion binaries, the
signed manifest files, and optional minisign verification. The manifest
signature check is mandatory for local-signature releases; a release is not
verified until `minisign -Vm` passes against `config/manifest-sign.pub`. The
script handles `.tar.zst` Debian payloads with a streaming zstandard reader
because published `.deb` members may omit an embedded content-size header.

For a demo-facing macOS release, also prove the installer path users see:

```bash
just install
test -d /Applications/Capsem.app
open -a Capsem
pgrep -x capsem-service
pgrep -x capsem-tray
```

`scripts/build-pkg.sh` must install `/Applications/Capsem.app` and carry a
fallback app copy in `/usr/local/share/capsem/Capsem.app` so postinstall cannot
report success while the GUI is missing. Relaunching `Capsem.app` must ask the
running service to ensure the tray via `/companions/tray/ensure`; spawning
`capsem-tray` directly bypasses the service parent guard and is not the product
path.
## Documentation site

The product website uses Astro Starlight. Docs live in `docs/src/content/docs/`.

### Writing style
Tight and to the point. One topic per page. Tables over prose for configs and test cases. No filler.

### Structure
- `docs/src/content/docs/<category>/<topic>.md`
- Categories: `security/`, `testing/`, `releases/`, `architecture/`
- Frontmatter: `title` and `description` required. `sidebar: { order: N }` for ordering.

### Release pages
- Path: `docs/src/content/docs/releases/<major>-<minor>.md` (hyphens, not dots)
- Each page consolidates all patch releases for that minor
- Higher `sidebar.order` = newer = listed first

### Dev workflow
```bash
cd site && pnpm run dev     # localhost:4321
cd site && pnpm run build   # Production build
```

### Keep docs in sync
When features change (settings, CLI flags, MCP tools, security invariants, benchmarks), update the corresponding doc page. When cutting a new minor, create a new release page.

### Update benchmarks before release

Run the host-side benchmarks to generate versioned data files and update the results page:

```bash
# Generate benchmarks/fork/data_{version}.json and benchmarks/lifecycle/data_{version}.json
uv run pytest tests/capsem-serial/test_lifecycle_benchmark.py -xvs

# Update docs/src/content/docs/benchmarks/results.md with new numbers
# (manual -- copy from the benchmark summary tables)
```

Benchmark data files in `benchmarks/` are committed to git for historical tracking. The `test_fork_benchmark` gates ensure fork stays under 500ms and images under 12MB -- these must pass before release.

## Changelog

Keep a Changelog format in `CHANGELOG.md`. Every user-visible change gets an entry under `## [Unreleased]` using: Added, Changed, Deprecated, Removed, Fixed, Security.

## Versioning

Binary and asset versions are **orthogonal**:

- **Binary**: `1.2.{unix_timestamp}` for the current release line -- auto-stamped by `just _stamp-version` on every `just install` and `just cut-release`. Set `CAPSEM_RELEASE_VERSION=x.y.z` when you need an exact preselected stamp.
- **Assets**: `YYYY.MMDD.patch` -- auto-derived by `gen_manifest.py` from the build date

Three files hold the binary version (kept in sync by `_stamp-version`): `Cargo.toml` (workspace), `crates/capsem-app/tauri.conf.json`, `pyproject.toml`.

The v2 manifest links them via `min_binary` (oldest binary for these assets) and `min_assets` (oldest assets for this binary). See `/asset-pipeline` for manifest format.

## Commits

1. Include `CHANGELOG.md` update in the same commit
2. Stage files explicitly (no `git add -A`)
3. Conventional messages: `feat:`, `fix:`, `chore:`, `docs:`
4. Author: Elie Bursztein <github@elie.net>
5. No `Co-Authored-By` trailers
6. Never stage private release material (`private/`, `capsem-private.zip`,
   `graphics.zip`, certificates, keys, tokens, or local-only demo credentials)
