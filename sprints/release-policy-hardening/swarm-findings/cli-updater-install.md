# CLI, Updater, and Install Findings

Status: completed, pending transfer into T0/T5/T9/T10/T11.

Agent: Hypatia (`019e1264-dd92-7c23-8767-a72c4f9ffc58`)

## Scope

- Setup/update manifest verification.
- Fresh install behavior.
- Tauri updater truthfulness.
- Version display and update UI/settings.
- Package install assumptions.

## Findings

- [ ] [P0] Clean installs can report success without a usable signed manifest.
  - Paths: `scripts/build-pkg.sh:63`, `scripts/deb-postinst.sh:31`,
    `crates/capsem/src/setup.rs:190`, `crates/capsem/src/setup.rs:121`,
    `crates/capsem-core/src/vm/boot.rs:165`,
    `tests/capsem-install/test_installed_layout.py:74`.
  - Detail: macOS package copies only `manifest.json`, not
    `manifest.json.minisig`; Linux postinstall creates assets dir but seeds no
    manifest; setup skips asset checks when manifest is missing and then marks
    install complete. Release boot expects verified manifests.
  - Proof: install tests must fail on missing `.deb` manifests instead of
    skipping; package payload checks must require `manifest.json` and
    `manifest.json.minisig`.
  - Sprint IDs: T0.1, T0.2, T0.3, T0.4, T10.1, T11.3.

- [ ] [P0] Linux packages omit required MCP helper binaries.
  - Paths: `.github/workflows/release.yaml:518`,
    `scripts/repack-deb.sh:34`, `scripts/deb-postinst.sh:34`,
    `scripts/simulate-install.sh:42`,
    `tests/capsem-install/conftest.py:89`,
    `crates/capsem-process/src/main.rs:331`,
    `crates/capsem-process/src/main.rs:728`,
    `crates/capsem-process/src/main.rs:730`.
  - Detail: Linux build/repack/install tests encode the stale six-binary
    contract while runtime expects `capsem-mcp-builtin` and
    `capsem-mcp-aggregator`.
  - Proof: `.deb` contents test requiring all eight binaries.
  - Sprint IDs: T5.1, T10.1, T11.1.

- [ ] [P1] Postinstall scripts suppress release-critical failures.
  - Paths: `scripts/pkg-scripts/postinstall:69`,
    `scripts/deb-postinst.sh:48`, `scripts/deb-postinst.sh:50`.
  - Detail: service install and setup run with `|| true`, so a package can
    exit 0 while setup, service registration, or asset preparation failed.
  - Proof: fresh `.pkg`/`.deb` install from empty home must fail loudly or
    persist deferred failure state.
  - Sprint IDs: T0.5, T10.1, T11.3.

- [ ] [P1] Setup/update/status trust unsigned manifests outside the boot
  verifier.
  - Paths: `crates/capsem/src/setup.rs:204`,
    `crates/capsem/src/update.rs:192`, `crates/capsem/src/main.rs:824`,
    `crates/capsem/src/main.rs:1048`,
    `crates/capsem-core/src/asset_manager.rs:249`,
    `tests/capsem-install/test_asset_download.py:231`.
  - Detail: these paths parse with `ManifestV2::from_json` instead of the
    verified loader; tests cover hash mismatch and 404s but not
    missing/tampered `.minisig`.
  - Proof: setup/update/status/doctor tests for missing/tampered signature.
  - Sprint IDs: T0.4, T10.2.

- [ ] [P1] Tauri updater is enabled but release workflow does not publish its
  feed, and the install model is wrong.
  - Paths: `crates/capsem-app/tauri.conf.json:30`,
    `crates/capsem-app/tauri.conf.json:47`,
    `crates/capsem-app/src/main.rs:350`,
    `crates/capsem-app/src/main.rs:175`,
    `.github/workflows/release.yaml:786`.
  - Detail: app launch always checks `latest.json`, then Tauri
    `download_and_install` updates only the app bundle; release upload has no
    `latest.json` path.
  - Proof: either disable updater and permissions, or publish verified
    full-install updater artifacts.
  - Sprint IDs: T0.6, T9.3, T10.1, T11.1.

- [ ] [P1] Post-release verification does not prove package-installed
  freshness.
  - Paths: `.github/workflows/release.yaml:876`,
    `.github/workflows/release.yaml:900`.
  - Detail: workflow skips binary E2E when no `.deb` is present and manually
    seeds `/tmp/capsem-home/assets/manifest.json`, bypassing package payload
    contract.
  - Proof: expand/install real package payloads and verify manifest, minisig,
    helper binaries, setup, update-assets, and status from empty home.
  - Sprint IDs: T0.7, T10.1, T11.3.

- [ ] [P2] App update UI/settings are disconnected from real behavior.
  - Paths: `frontend/src/lib/components/settings/SettingsSection.svelte:139`,
    `frontend/src/lib/api.ts:806`,
    `crates/capsem-service/src/main.rs:4560`,
    `config/defaults.toml:20`.
  - Detail: Settings renders a Check now button without handler; API calls
    `/update/check`; service has no route; `app.auto_update` is ignored by app
    startup.
  - Sprint IDs: T0.6, T10.3.

- [ ] [P2] User-facing version/update truth is stale or misleading.
  - Paths: `frontend/src/lib/components/shell/SettingsPage.svelte:345`,
    `crates/capsem/src/main.rs:85`, `crates/capsem/src/update.rs:62`,
    `crates/capsem/src/update.rs:179`.
  - Detail: About displays `0.1.0-dev`; CLI/help/notices imply binary
    self-update exists, while `run_update` says it is not wired.
  - Proof: release text and UI/CLI copy must match actual update behavior.
  - Sprint IDs: T0.6, T9.1, T9.3, T10.6.

- [ ] [P0] Do not tag yet.
  - Paths: `sprints/release-policy-hardening/tracker.md:14`,
    `sprints/release-policy-hardening/T11-full-release-gate.md:62`.
  - Detail: T0, T5, T9, T10, and T11 are not started/release-blocking and the
    tree is dirty.
  - Sprint IDs: T11.4, T11.5.

## Tests Not Run

- Static no-edit investigation only; no test suite was run.
