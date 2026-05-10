# T0: Release Artifacts and Install Bootability

## Objective

Make every published package installable from a clean machine and honest about
update support. A release package must carry or fetch a verified manifest,
must not report success while boot-critical setup failed, and must be tested
from the package payload rather than from a hand-prepared development layout.

## Owned Files

- `.github/workflows/release.yaml`
- `scripts/build-pkg.sh`
- `scripts/pkg-scripts/postinstall`
- `scripts/deb-postinst.sh`
- `scripts/repack-deb.sh`
- `scripts/check-release-workflow.sh`
- `crates/capsem/src/setup.rs`
- `crates/capsem/src/update.rs`
- `crates/capsem/src/main.rs`
- `crates/capsem-service/src/main.rs`
- `crates/capsem-app/tauri.conf.json`
- `crates/capsem-app/Cargo.toml`
- `crates/capsem-app/capabilities/default.json`
- `crates/capsem-app/src/main.rs`
- `frontend/src/lib/components/settings/SettingsSection.svelte`
- `frontend/src/lib/components/shell/SettingsPage.svelte`
- `frontend/src/lib/api.ts`
- `tests/capsem-install/*`

## Findings

- [P0] `scripts/build-pkg.sh:63` copies only `manifest.json` into the macOS
  package. Release boot requires a sibling `manifest.json.minisig`.
- [P0] The unified manifest is signed in create-release after the `.pkg` is
  built. If the packaged manifest is expected to be the final manifest with
  package file hashes, there is a self-reference problem because changing the
  package payload changes the package hash.
- [P1] The macOS package is built from `vm-assets-arm64`, while the unified
  two-arch manifest is generated later.
- [P1] Post-release verification seeds `/tmp/capsem-home/assets/manifest.json`
  manually before running `capsem update --assets`, so it does not prove that
  packages install manifests correctly.
- [P1] Post-release verification can skip binary E2E when no `.deb` is present.
- [P0] `scripts/deb-postinst.sh` creates `~/.capsem/assets` but seeds no
  `manifest.json` or `manifest.json.minisig`. `capsem setup` treats a missing
  manifest as a skipped asset check and later sets `install_completed = true`.
- [P1] `setup`, `update --assets`, service startup, status, and doctor asset
  checks parse manifests directly instead of using the verified manifest loader
  used by release boot.
- [P1] macOS and Linux postinstall scripts suppress setup/service registration
  failures, letting packages exit 0 while leaving Capsem non-bootable.
- [P1] Tauri updater points at `latest.json` and checks on launch, but release
  uploads no `latest.json` or updater archives.
- [P1] Tauri `download_and_install` would update only the app bundle, not the
  companion binaries, LaunchAgent/service setup, package scripts, or asset
  manifest behavior that the custom `.pkg` owns.
- [P2] The frontend update button has no working backend or Tauri IPC path, and
  `auto_check_updates` does not control the launch-time Tauri updater check.
- [P3] Settings About hardcodes `0.1.0-dev`.
- [P2] `scripts/check-release-workflow.sh` checks the wrong signing key family
  for manifest signing.
- [P3] Release attestation includes rootfs only, not kernel, initrd, manifest,
  or manifest signature.

## Task List

### T0.1 Define the Manifest Contract

- [ ] Decide whether the package payload manifest is the final published
  manifest or a signed asset-compatibility snapshot.
- [ ] If it is an asset snapshot, assert its `assets` section matches the
  published manifest and document that package file hashes live only in the
  published release manifest.
- [ ] If it is the final manifest, design a two-stage package/signing flow that
  avoids self-referential package hashes.
- [ ] Record the chosen contract in this file, release docs, and release CI
  comments.

### T0.2 MacOS Package Manifest and Signature

- [ ] Download both `vm-assets-arm64` and `vm-assets-x86_64` before `.pkg`
  construction.
- [ ] Generate a unified two-arch package payload manifest before
  `scripts/build-pkg.sh`.
- [ ] Preserve `date`, `deprecated`, and `min_assets` when adding binary file
  metadata.
- [ ] Sign the package payload manifest before packaging.
- [ ] Copy both `manifest.json` and `manifest.json.minisig` in
  `scripts/build-pkg.sh`.
- [ ] Add CI assertions that expand the package, find both files, verify the
  minisign signature, and assert both arch maps exist.

### T0.3 Linux Package Manifest and Signature

- [ ] Seed signed `manifest.json` and `manifest.json.minisig` into the `.deb`,
  or fetch and verify them during postinstall before setup can complete.
- [ ] Make `capsem setup` fail or remain incomplete in installed layouts when a
  signed manifest is missing or invalid.
- [ ] Update install tests to require manifest files in installed `.deb` mode.
- [ ] Add a fresh-home `.deb` proof that runs setup, update-assets, and status
  without any manually seeded manifest.

### T0.4 Verified Manifest Consumers

- [ ] Use `load_verified_manifest_for_assets` or equivalent verified loading in
  setup.
- [ ] Use verified loading in `capsem update --assets`.
- [ ] Use verified loading in service startup asset status checks.
- [ ] Use verified loading in CLI status and doctor asset checks.
- [ ] Add negative tests for missing signature, tampered manifest, and unsigned
  local manifest in every user-facing diagnostic path.

### T0.5 Package Failure Semantics

- [ ] Decide which postinstall failures can defer and which must fail the
  installer.
- [ ] Stop suppressing release-critical `capsem install` and `capsem setup`
  failures, or persist explicit deferred failure state.
- [ ] Retry deferred setup loudly on first run.
- [ ] Surface deferred install state in `capsem status` or setup output.

### T0.6 Desktop Updater Strategy

- [ ] Disable/remove Tauri updater config and frontend updater affordances for
  this release, or publish and verify the exact `latest.json` plus updater
  artifacts it requires.
- [ ] If updater remains enabled, make it update the full Capsem install,
  including companion binaries and service/package state.
- [ ] Honor `auto_check_updates` for launch-time checks.
- [ ] Wire `Check now` to a real supported backend or Tauri IPC path.
- [ ] Display the stamped app version instead of `0.1.0-dev`.
- [ ] Add a preflight assertion that configured updater endpoints imply matching
  uploaded artifacts.
- [ ] Reduce Tauri updater permissions if updater is disabled.

### T0.7 Release Preflight and Post-Release Proof

- [ ] Update manifest-signing preflight to use the manifest signing key and
  prove it matches `config/manifest-sign.pub`.
- [ ] Add preflight checks for manifest signing secrets.
- [ ] Replace post-release verification's manually seeded manifest path with a
  true install-from-package check for every package that was published.
- [ ] Make optional package absence explicit: fail when an expected package is
  missing or record a release-blocking reason before continuing.
- [ ] Include kernel, initrd, rootfs, manifest, and manifest signature in
  provenance attestation where GitHub Actions supports it.

## Proof Matrix

| Category | Required proof |
|---|---|
| Unit/contract | setup/update/service/status/doctor reject missing or invalid manifest signatures. |
| Functional | expanded `.pkg` and `.deb` contain manifest/signature and expected package files. |
| Adversarial | tampered manifest, missing minisig, missing package helper, and suppressed setup failure all fail loudly. |
| E2E/install | clean macOS `.pkg` and Linux `.deb` installs can run setup/update/status from empty `CAPSEM_HOME`. |
| VM | at least one clean package install boots `capsem-doctor` before tagging. |
| Release CI | post-release verification starts from package output, not manually seeded manifest files. |

## Verification

- [ ] `pkgutil --expand-full packages/Capsem-*.pkg /tmp/capsem-pkg`
- [ ] `find /tmp/capsem-pkg -name manifest.json -o -name manifest.json.minisig`
- [ ] `minisign -Vm /tmp/capsem-pkg/**/manifest.json -p config/manifest-sign.pub`
- [ ] `cargo test -p capsem-core load_verified_manifest_bails_when_sig_required_but_missing -- --nocapture`
- [ ] Add named negative tests for unsigned, missing-signature, and tampered
  manifests across setup/update/status/doctor.
- [ ] `cargo test -p capsem-app`
- [ ] `cd frontend && pnpm run check`
- [ ] `cd frontend && pnpm run test`
- [ ] `dpkg-deb --contents target/release/bundle/deb/*.deb | rg 'manifest\\.json(\\.minisig)?'`
- [ ] Fresh Linux `.deb` install from empty `CAPSEM_HOME`: `capsem setup
  --non-interactive`, `capsem update --assets`, `capsem status`, and
  `capsem run capsem-doctor`.
- [ ] Clean macOS `.pkg` install from empty `CAPSEM_HOME`: `capsem setup
  --non-interactive`, `capsem update --assets`, `capsem status`, and
  `capsem run capsem-doctor`.
- [ ] Post-release verification starts from an empty `CAPSEM_HOME` and package
  payload only.
- [ ] `find target/release/bundle -maxdepth 4 -type f | sort | rg 'latest\\.json|\\.tar\\.gz|\\.sig|\\.pkg|\\.deb'`

## Exit Criteria

- [ ] No release package can be published without a signed manifest.
- [ ] Package verification fails if the packaged manifest has only one arch.
- [ ] Setup/update/status/doctor trust the same verified manifest rules boot
  uses.
- [ ] Update UI either works against real release artifacts or is hidden.
- [ ] Manual or CI clean package install proof is recorded before tagging.
