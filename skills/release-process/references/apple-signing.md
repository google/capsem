# Apple Code Signing and CI Secrets

Reference for /release-process: p12 encryption, notarization, signing identities, CI secrets, Cloudflare prerequisites.


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

When a release gate needs user action, say so plainly. Do not describe an
Apple agreement 403 as a transient notarization error or a credential problem.
Tell the user:

- what blocked the release (`notarytool history` returned 403 because an Apple
  agreement is missing or expired)
- why the agent cannot fix it (only the Apple Account Holder can accept it)
- exactly what to do (sign in to the two Apple pages above and accept any
  pending agreements or banking/tax terms)
- when to retry (after the agreement is accepted and 1-5 minutes have passed)
- what was intentionally not done (no release commit/tag/push happened)

If a timed retry is useful, offer or create a heartbeat retry. Keep the release
paused until `notarytool history` succeeds locally.

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
| `CLOUDFLARE_ACCOUNT_ID` | Cloudflare account that owns the Pages project serving `release.capsem.org` |
| `CLOUDFLARE_API_TOKEN` | API token allowed to deploy the Pages project serving `release.capsem.org` |

CI secrets are the source of truth for release signing. Local backups in
`private/apple-certificate/` and `private/tauri/` are useful for local preflight
and packaging checks, but they are gitignored and must never be staged.

### Release-channel Cloudflare prerequisites

Before running a live binary or VM asset channel deploy, create or verify the
Cloudflare Pages project serving `release.capsem.org`, attach the `release.capsem.org`
custom domain, and configure `CLOUDFLARE_ACCOUNT_ID` plus
`CLOUDFLARE_API_TOKEN` in GitHub Actions secrets. `release-channel.yaml` fails
before deploy if either secret is missing or
`scripts/check-cloudflare-pages-project.py` cannot see the Pages project through
the configured account/token, then runs `scripts/check-release-site-contract.py`
and smokes `https://release.capsem.org/`, `/channels.json`, and the channel
manifest through the public custom domain after Cloudflare publishes the
generated site. Live VM asset releases use the same project preflight before
the expensive asset build matrix starts.
