# Apple Developer Signing Setup Checklist

Required before the first CI release. All steps are manual (portal/Xcode).

## 1. Apple Developer Program

- [ ] Enroll at https://developer.apple.com/programs/ ($99/year)
- [ ] Apple ID must have two-factor authentication enabled

## 2. Developer ID Application Certificate

- [ ] Create in Xcode: Settings > Accounts > Manage Certificates > "+" > Developer ID Application
  - Or via https://developer.apple.com > Certificates > "+" > Developer ID Application
- [ ] Export as `.p12` from Keychain Access (set an export password)
- [ ] Set GitHub secret `APPLE_CERTIFICATE`: run `base64 -i cert.p12 | pbcopy`, paste as secret value
- [ ] Set GitHub secret `APPLE_CERTIFICATE_PASSWORD`: the export password

## 3. App Store Connect API Key (for notarization)

- [ ] Go to https://appstoreconnect.apple.com/access/integrations/api
- [ ] Create a new key with "Developer" role
- [ ] Download the `.p8` file (only available once -- save it)
- [ ] Set GitHub secret `APPLE_SIGNING_IDENTITY`: `"Developer ID Application: Your Name (TEAMID)"`
  - Find TEAMID in Xcode or at https://developer.apple.com/account > Membership
- [ ] Set GitHub secret `APPLE_API_ISSUER`: the Issuer ID shown on the API keys page
- [ ] Set GitHub secret `APPLE_API_KEY`: the Key ID of the key you created
- [ ] Set GitHub secret `APPLE_API_KEY_PATH`: tauri-action writes the `.p8` here automatically

## 4. Tauri Updater Signing Key

- [ ] Generate: `cargo tauri signer generate -w ~/.tauri/capsem.key`
- [ ] Set GitHub secret `TAURI_SIGNING_PRIVATE_KEY`: contents of `~/.tauri/capsem.key`
- [ ] Set GitHub secret `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: the password you chose
- [ ] Verify: public key in `crates/capsem-app/tauri.conf.json` matches the generated public key

## 5. Verify

- [ ] All 8 GitHub secrets are set: `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_API_ISSUER`, `APPLE_API_KEY`, `APPLE_API_KEY_PATH`, `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
- [ ] Tag a test release (`git tag v0.8.0-rc1 && git push origin v0.8.0-rc1`)
- [ ] Verify release.yaml runs: certificate imports, DMG is signed, notarization succeeds
- [ ] Verify `latest.json` updater manifest is uploaded to the release
- [ ] Test auto-update detection from a previously installed version

## Notes

- `tauri-action@v1` may handle certificate import automatically if `APPLE_CERTIFICATE` and `APPLE_CERTIFICATE_PASSWORD` are set. The explicit import step in `release.yaml` is a fallback.
- The `.p8` key file should be stored securely outside the repo. Only the Key ID and Issuer ID go into GitHub secrets.
- The Tauri updater public key is checked into `tauri.conf.json` -- it is NOT secret.
