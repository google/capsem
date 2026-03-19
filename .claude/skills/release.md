# Release Skill

Use this skill when preparing, debugging, or executing a Capsem release.

## Pre-Release Checklist

Run locally before pushing a release tag:

```bash
just doctor                    # Checks all tools are installed
just build-assets              # Rebuild VM assets if needed (needs docker/podman)
scripts/preflight.sh           # Validates Apple certs for CI
just full-test                 # Unit tests + capsem-doctor + integration + bench
```

## Cutting a Release

Releases are CI-only -- no local `just release`. Push a tag to trigger the pipeline.

1. **Bump version** in both places:
   - `workspace.package.version` in root `Cargo.toml`
   - `version` in `crates/capsem-app/tauri.conf.json`
2. **Update CHANGELOG.md**: move `[Unreleased]` items into `[X.Y.Z] - YYYY-MM-DD`
3. **Create news post**: `site/src/pages/news/<version>.md` (e.g. `0.9.0.md`) summarizing what changed, using `layout: ../../layouts/Doc.astro`. Add a matching entry to the `releases` array in `site/src/pages/news/index.astro`.
4. **Update benchmarks** (if performance-relevant changes): run `just bench` and update the numbers in `site/src/pages/documentation/testing/benchmarks.md`. Always update `lastUpdated` in its frontmatter when numbers change.
5. **Run preflight**: `scripts/preflight.sh` (validates Apple certs for CI)
6. **Check release workflow**: `just check-release` (verifies tools, key format, manifest signing, version sync)
7. **Run tests**: `just full-test`
8. **Commit**: `git commit -m "release: vX.Y.Z"`
9. **Tag**: `git tag vX.Y.Z`
10. **Push**: `git push origin main --tags`
11. **Publish**: `just release` -- runs check-release, waits for CI, triggers publish workflow

CI pipeline: preflight -> build-assets -> test -> build-app (sign + notarize + artifact upload).
`just release` then downloads the artifacts and creates the GitHub release locally (CI can't -- org restricts GITHUB_TOKEN to read-only).

## CI Pipeline (release.yaml)

| Job | Runner | Purpose |
|-----|--------|---------|
| `preflight` | macos-14 | Fail-fast: verify Apple cert imports, Tauri key exists |
| `build-assets` | ubuntu-24.04-arm | Build VM assets (kernel, initrd, rootfs) via Docker |
| `test` | macos-14 | Unit tests, cross-compile check, frontend build |
| `build-app` | macos-14 | Tauri build, codesign, notarize, DMG, upload CI artifacts |

## Apple Code Signing

### Certificate chain
- Developer ID Application certificate (`.p12`) -> `APPLE_CERTIFICATE` secret (base64)
- App Store Connect API key (`.p8`) -> `APPLE_API_KEY_PATH` secret (file contents)
- Signing identity string -> `APPLE_SIGNING_IDENTITY` secret

### p12 encryption format (CRITICAL GOTCHA)

macOS Keychain only accepts legacy PKCS12 encryption (3DES/SHA1). OpenSSL 3.x creates PBES2/AES-256-CBC by default. Keychain rejects it with a misleading "wrong password?" error.

**How to tell**: `openssl pkcs12 -in cert.p12 -info -nokeys -nocerts -passin pass:PWD 2>&1 | head -5`
- `PBES2` = modern (broken on macOS)
- `pbeWithSHA1And3-KeyTripleDES-CBC` = legacy (works)

**How to fix**:
```bash
scripts/fix_p12_legacy.sh
# Then upload:
gh secret set APPLE_CERTIFICATE < private/apple-certificate/capsem-b64.txt
```

**Manual fix** (if script unavailable):
```bash
openssl pkcs12 -in cert.p12 -passin pass:PWD -nodes -out combined.pem
openssl pkcs12 -export -in combined.pem -out cert-legacy.p12 -passout pass:PWD \
    -certpbe PBE-SHA1-3DES -keypbe PBE-SHA1-3DES -macalg sha1
```

Do NOT use `-legacy` flag alone -- it uses RC2-CBC for certs which OpenSSL itself can't read back. Explicitly set `-certpbe PBE-SHA1-3DES -keypbe PBE-SHA1-3DES -macalg sha1`.

## Tauri Updater Signing

- Private key: `TAURI_SIGNING_PRIVATE_KEY` secret (minisign format)
- Public key: checked into `crates/capsem-app/tauri.conf.json` (not secret)
- Generate new: `cargo tauri signer generate -w ~/.tauri/capsem.key`

## CI Secrets Reference

| Secret | Purpose |
|--------|---------|
| `APPLE_CERTIFICATE` | Base64 `.p12` -- MUST be legacy 3DES format |
| `APPLE_CERTIFICATE_PASSWORD` | Password for the `.p12` |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: Elie Bursztein (L8EGK4X86T)` |
| `APPLE_API_ISSUER` | App Store Connect API issuer UUID |
| `APPLE_API_KEY` | App Store Connect API key ID |
| `APPLE_API_KEY_PATH` | Contents of `.p8` private key file |
| `TAURI_SIGNING_PRIVATE_KEY` | Tauri updater minisign private key |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Password for Tauri signing key |

Local backups: `private/apple-certificate/` and `private/tauri/` (gitignored).

## Preflight Script (scripts/preflight.sh)

Non-destructive validation of release prerequisites. Run before every release.

Checks:
- Required CLI tools (openssl, codesign, cargo, pnpm, node, gh)
- Rust aarch64-unknown-linux-musl target
- Apple p12 encryption format + keychain import test
- Base64 file matches p12 on disk
- Notarization credentials (.p8 key, API Key ID, Issuer ID) + live `notarytool history` test

To add a new check: add a `check_*` function to `scripts/preflight.sh`.

## Debugging Release Failures

### "Import Apple certificate" fails
1. Check p12 format: `openssl pkcs12 -in cert.p12 -info ...` -- look for PBES2
2. Fix: `scripts/fix_p12_legacy.sh` + re-upload secret
3. Verify: `scripts/preflight.sh`

### Notarization fails or hangs
- CI uses `--skip-stapling` so `tauri build` submits but does not wait for Apple's response
- First-time notarization can take hours -- `--skip-stapling` prevents this from blocking CI
- Check `.p8` key is valid and has "Developer" role
- Check `APPLE_API_ISSUER` and `APPLE_API_KEY` match the key
- Verify team membership is active ($99/year Apple Developer Program)
- Verify credentials locally: `xcrun notarytool history --key private/apple-certificate/capsem.p8 --key-id KEY_ID --issuer ISSUER_ID`
- Check notarization status after CI: `xcrun notarytool log <submission-id> --key ... --key-id ... --issuer ...`

### DMG not signed
- Check `APPLE_SIGNING_IDENTITY` matches certificate CN exactly
- Try: `security find-identity -v -p codesigning` to see available identities

### Tauri build fails
- Check VM assets are present in `assets/` (downloaded from build-assets job)
- Check `pnpm install --frozen-lockfile` passes (lockfile in sync)
- Check frontend builds: `cd frontend && pnpm run build`

## Post-Release Verification

After pushing a tag and CI completes, verify the release using `gh` CLI:

### Monitor the CI pipeline
```bash
gh run list --branch main -L 1          # Find the run triggered by the tag
gh run watch <run-id>                    # Live tail the CI run
gh run view <run-id> --log-failed       # Debug failures
```

If any job fails: fix locally, create a NEW commit, bump the version (e.g. 0.9.0 -> 0.9.1), tag the new version, and push. **Never delete or move a tag** -- tags are immutable references. Always tag forward.

### After CI completes successfully

**1. Download CI artifacts and create the release:**
```bash
# Find the run ID
gh run list --workflow=release.yaml -L 1

# Download built artifacts
gh run download <run-id> -n release-artifacts -D /tmp/release-artifacts

# Create the GitHub release with all assets
gh release create vX.Y.Z /tmp/release-artifacts/* \
  --title "Capsem vX.Y.Z" \
  --notes "See CHANGELOG.md for details."
```

**2. Verify the release:**
```bash
# Check assets are listed
gh release view vX.Y.Z

# Verify manifest.json
gh release download vX.Y.Z --pattern manifest.json -D /tmp/verify-release
cat /tmp/verify-release/manifest.json | python3 -m json.tool

# Verify DMG mounts
gh release download vX.Y.Z --pattern '*.dmg' -D /tmp/verify-release
hdiutil attach /tmp/verify-release/Capsem*.dmg -nobrowse -readonly
ls /Volumes/Capsem*/
hdiutil detach /Volumes/Capsem*

# Verify /releases/latest points to new version
curl -fsSL "https://api.github.com/repos/google/capsem/releases/latest" | grep tag_name
```
