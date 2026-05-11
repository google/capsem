# CI Packaging and Release Artifact Findings

Status: completed; transferred to T7 FD10 and owner rows in
T0/T1/T5/T10/T11/T12. Downstream implementation remains open.

Agent: Bernoulli (`019e1269-a192-72f0-a6ee-67e338b017aa`)

## Scope

- Release artifact production.
- Manifest/minisig inclusion.
- macOS `.pkg` payload.
- Linux `.deb` payload.
- Helper binaries.
- `latest.json` and updater artifacts.
- Post-release verification.
- CI proof.

## Findings

- [ ] [P0] `.pkg` ships the wrong manifest contract.
  - Release impact: a fresh macOS package can install an arm64-only unsigned
    manifest while release boot requires a signed manifest.
  - Paths: `.github/workflows/release.yaml:227`, `scripts/build-pkg.sh:60`,
    `scripts/pkg-scripts/postinstall:48`,
    `crates/capsem-core/src/vm/boot.rs:164`.
  - Detail: macOS CI downloads only `vm-assets-arm64`, generates an arm64-only
    manifest, packages it before the final unified manifest is signed, and
    `build-pkg.sh` copies no `manifest.json.minisig`.
  - Proof: expand `.pkg`; find both manifest files; run
    `minisign -Vm ... -p config/manifest-sign.pub`; assert both arch maps;
    clean `.pkg` install and run setup/update/status/doctor.
  - Sprint IDs: T0.1, T0.2, T10.1, T11.3.

- [ ] [P0] `.deb` can install successfully with no signed manifest.
  - Release impact: Linux install can mark setup complete while later boot
    lacks a verified manifest.
  - Paths: `scripts/repack-deb.sh:32`, `scripts/deb-postinst.sh:30`,
    `crates/capsem/src/setup.rs:190`, `crates/capsem/src/setup.rs:121`,
    `.github/workflows/release.yaml:897`.
  - Detail: repack adds binaries/postinst only; postinst creates
    `~/.capsem/assets` but seeds no manifest/minisig and suppresses setup
    failures; setup skips missing manifest then marks `install_completed=true`.
  - Proof: `dpkg-deb --contents ... | rg 'manifest\\.json(\\.minisig)?'`,
    fresh `.deb` install from empty `CAPSEM_HOME` without manual manifest
    seeding, then setup/update/status/doctor.
  - Sprint IDs: T0.3, T0.5, T0.7, T10.1, T11.3.

- [x] [P0] Linux `.deb` omits runtime MCP helpers.
  - Release impact: installed Linux release can lose MCP helper functionality.
  - Paths: `.github/workflows/release.yaml:516`,
    `scripts/repack-deb.sh:34`, `scripts/deb-postinst.sh:34`,
    `scripts/simulate-install.sh:42`,
    `crates/capsem-process/src/main.rs:330`,
    `crates/capsem-process/src/main.rs:725`,
    `tests/test_repack_deb.py:27`.
  - Proof: require eight binaries in repack/install tests, inspect `.deb`
    contents for both helpers, and run MCP tool discovery from installed
    `.deb`.
  - Sprint IDs: T5.1, T10.1, T11.1.
  - Transfer status: resolved in T5; generated package and installed MCP tool
    discovery proof remains T10/T11.

- [ ] [P1] CI can publish while Linux packaging/rootfs validation failed.
  - Release impact: release can publish without expected Linux packages or
    validated rootfs.
  - Paths: `.github/workflows/release.yaml:398`,
    `.github/workflows/release.yaml:584`,
    `.github/workflows/release.yaml:653`,
    `.github/workflows/release.yaml:876`.
  - Detail: `build-app-linux` is `continue-on-error`; create-release treats
    `.deb` as optional; post-release binary E2E exits 0 when no deb exists.
  - Proof: explicit expected package matrix; fail release when expected `.deb`
    is absent or record release-blocking owner before publish.
  - Sprint IDs: T0.7, T1.5, T10.1, T11.1.

- [x] [P1] Rootfs validation misses required guest binaries and is not a hard
  gate.
  - Release impact: release assets can be missing binaries required by
    `capsem-init`.
  - Paths: `.github/workflows/release.yaml:485`,
    `src/capsem/builder/docker.py:28`,
    `guest/artifacts/capsem-init:311`,
    `guest/artifacts/capsem-init:352`.
  - Detail: workflow checks only `capsem-pty-agent`, `capsem-net-proxy`,
    `capsem-mcp-server`, `capsem-doctor`, `capsem-bench`, `snapshots`; canonical
    guest binaries also include `capsem-dns-proxy` and `capsem-sysutil`.
  - Proof: derive validation from `GUEST_BINARIES` plus rootfs scripts, mount
    every release rootfs in a hard-gated job, fail create-release on missing
    artifacts.
  - Sprint IDs: T1.5, T5.4, T10.2.
  - Transfer status: resolved in T1/T5; generated rootfs proof remains T10/T11.

- [ ] [P1] Release manifest binary metadata is overwritten, and promised
  regression test is absent.
  - Release impact: compatibility metadata such as `min_assets` can disappear
    from published manifest, affecting asset selection for binaries.
  - Paths: `.github/workflows/release.yaml:660`,
    `src/capsem/builder/docker.py:733`,
    `tests/test_release_workflow_policy.py`.
  - Detail: create-release now preserves generated `date`, `deprecated`, and
    `min_assets` while adding `version/files`.
  - Proof: run
    `tests/test_release_workflow_policy.py::test_create_release_preserves_binary_metadata`.
  - Sprint IDs: T1.1, T10.2.

- [ ] [P1] Updater is enabled but release artifacts do not satisfy it.
  - Release impact: app can auto-check a missing `latest.json`, and the UI can
    offer an update path that cannot work.
  - Paths: `crates/capsem-app/tauri.conf.json:30`,
    `crates/capsem-app/tauri.conf.json:44`,
    `crates/capsem-app/src/main.rs:138`,
    `.github/workflows/release.yaml:381`,
    `.github/workflows/release.yaml:763`,
    `frontend/src/lib/api.ts:805`,
    `frontend/src/lib/components/settings/SettingsSection.svelte:139`.
  - Proof: either disable updater and permissions, or upload/verify exact
    `latest.json` plus updater archives/signatures.
  - Sprint IDs: T0.6, T10.1, T11.1.

- [ ] [P1] Manifest-signing preflight checks the wrong key family.
  - Release impact: local/CI preflight can pass while the manifest signing key
    does not match the baked verifier key.
  - Paths: `.github/workflows/release.yaml:50`,
    `.github/workflows/release.yaml:694`,
    `scripts/check-release-workflow.sh:23`,
    `config/manifest-sign.pub:1`.
  - Detail: CI signs with `MINISIGN_SECRET_KEY`, but preflight only checks
    Tauri signing key; local script signs manifest with `private/tauri/capsem.key`.
  - Proof: preflight verifies manifest secret presence and
    `minisign -Vm release-artifacts/manifest.json -p config/manifest-sign.pub`.
  - Sprint IDs: T0.7, T11.1.

- [ ] [P2] Post-release proof does not verify package payloads, manifest
  signatures, or full provenance.
  - Release impact: live release verification can pass while packages contain
    wrong/missing payloads.
  - Paths: `.github/workflows/release.yaml:834`,
    `.github/workflows/release.yaml:897`,
    `.github/workflows/release.yaml:703`.
  - Detail: live verification downloads only `manifest.json`, manually seeds it
    into `CAPSEM_HOME`, and attests only pkg/deb/rootfs, not
    kernel/initrd/manifest/minisig.
  - Proof: verify published `manifest.json.minisig`, expand/install every
    package payload, and include kernel/initrd/manifest/minisig in provenance
    where supported.
  - Sprint IDs: T0.7, T10.7, T11.4.

## Tests Not Run

- Static release-policy pass only; no tests were run.

## T5.1 Execution Audit, 2026-05-10

Agent: Descartes (`019e1312-4d46-7153-b010-aadc111f3797`)

Status: completed; findings captured for T5.1.

### Findings

- [x] [P2] Linux `.deb` contents proof is too weak.
  - Release impact: CI can pass if any one of the listed files appears in the
    package contents; it does not independently prove both MCP helper binaries
    and both signed manifest files are present.
  - Paths: `.github/workflows/release.yaml:550`.
  - Required proof: make the workflow validate each required payload path
    independently, and keep static workflow tests requiring the helper names.
  - Sprint IDs: T5.1, T10.1.
  - Transfer status: resolved in T5; generated package payload inspection
    remains T10/T11.

### Confirmed Fixed By Current Worktree

- macOS `.pkg` builds, signs, packages, and postinstalls
  `capsem-mcp-aggregator` and `capsem-mcp-builtin`.
- Linux `.deb` builds, repacks, and postinstalls both MCP helpers.
- Simulated installs and install-test expected binary lists use the eight-binary
  contract.
- Package script tests cover the helper binaries and signed manifest files.

### Tests Run

- `uv run pytest tests/test_package_scripts.py tests/test_repack_deb.py -q`
  - Result: `3 passed, 6 skipped`.

### Required T5.1 Proof Set

- `uv run pytest tests/test_package_scripts.py tests/test_repack_deb.py -q`
- `uv run pytest tests/capsem-install/test_installed_layout.py tests/capsem-install/test_smoke.py tests/capsem-install/test_reinstall.py -q`
- After building artifacts:
  `dpkg-deb --contents target/release/bundle/deb/*.deb | rg 'capsem-mcp-(aggregator|builtin)'`
- After building pkg:
  `pkgutil --expand-full packages/Capsem-*.pkg /tmp/capsem-pkg && find /tmp/capsem-pkg -type f | rg 'capsem-mcp-(aggregator|builtin)'`
