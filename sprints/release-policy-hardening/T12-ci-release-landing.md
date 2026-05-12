# T12: CI Green Release Landing

## Objective

Land the `v1.1.1778542197` release only after T11 has produced a locally installed,
Elie-verified release candidate. T12 owns the tag, CI run, live GitHub release
assets, post-publish verification, and final "release landed" evidence.

No product code changes belong in T12. If T12 finds a product, package, docs,
or workflow bug, stop the release, reopen the owning track, and rerun T10/T11
after the fix.

## Owned Files

- `.github/workflows/release.yaml`
- `scripts/preflight.sh`
- `scripts/check-release-workflow.sh`
- `CHANGELOG.md`
- `LATEST_RELEASE.md`
- `docs/src/content/docs/releases/1-1.md`
- `sprints/release-policy-hardening/tracker.md`
- release commit, tag, CI run, and GitHub release assets

## Findings

- [P0] The sprint previously stopped at a local release hold; it did not have a
  final sprint that waits for CI green and verifies live release assets.
- [P0] The target release line is `1.1.1778542197`; T12 must fail if tag, changelog,
  binary metadata, release page, or published assets still claim the old
  `1.0.1778378133` release.
- [P1] Post-release verification must download published assets and inspect
  package payloads. A local package proof from T11 is necessary but not
  sufficient.

## Swarm Transfer Tracker

| Source | Priority | Owner task | Required transfer point | Required proof |
|---|---:|---|---|---|
| FD02 docs-release-metadata | P1 | T12.4, T12.5 | Release artifact truth from docs/release notes must match live published assets. | `gh release view/download` output and package payload inspection are recorded. |
| FD10 ci-packaging | P1/P2 | T12.3, T12.4 | Live release verification must not pass while packages/manifests/signatures/helpers/provenance are missing. | CI green criteria and live asset verification include manifest/minisig, `.pkg`, `.deb`, helpers, rootfs, and provenance where supported. |
| FD11 verification-architecture | P1 | T12.4 | T11 lacks post-publish release verification owner; T12 owns it. | T12.4 downloads and verifies live assets after CI. |
| FD13 ci-release-landing-1-1 | P0 | T12.1, T12.2, T12.5 | T12 must be represented consistently and own waiting for CI green plus live asset verification. | Tracker, plan, T11 handoff, and T12 evidence ledger agree before release is marked landed. |
| FD13 ci-release-landing-1-1 | P0 | T12.3 | Linux release publication must be release-blocking, not best-effort. | CI fails before `create-release` if Linux package, helper validation, or rootfs validation fails. |
| FD13 ci-release-landing-1-1 | P0 | T12.3 | Linux CI must build and validate MCP helper binaries in package payloads. | `dpkg-deb -c` proof from CI/live assets includes `capsem-mcp-aggregator` and `capsem-mcp-builtin`. |
| FD13 ci-release-landing-1-1 | P1 | T12.3 | Updater configuration must match publish workflow. | CI publishes/verifies updater artifacts or confirms updater is disabled/hidden for release. |
| FD13 ci-release-landing-1-1 | P1 | T12.1 | Local release-check scripts must cover CI landing invariants before tag. | T11 preflight/check-release output is recorded as T12 precondition. |
| FD14 swarm-transfer-closeout | P0 | T12.1, T12.5 | T12 is partially introduced but must be owned consistently across plan/tracker/T7/T11. | Release-control language review shows the T12 handoff is current outside historical finding docs. |

## Task List

### T12.1 Pre-Tag Readiness

- [ ] Confirm T11 is signed off by Elie in `tracker.md`.
- [ ] Confirm T9 recorded the exact `1.1.1778542197` version and all version files
  match it.
- [ ] Confirm `CHANGELOG.md`, `LATEST_RELEASE.md`, and
  `docs/src/content/docs/releases/1-1.md` describe the actual shipped scope.
- [ ] Confirm no active swarm agents or `In progress` finding docs remain.
- [ ] Confirm `git status --short` has only intentional release files.
- [ ] Confirm tag `v1.1.1778542197` does not already exist locally or remotely.

### T12.2 Tag and CI Run

- [ ] If `just cut-release` has been updated to produce the exact T9 version,
  use it and record the tag it creates.
- [ ] Otherwise, create the release commit and immutable tag manually, push the
  branch and `v1.1.1778542197` tag, then run `just release v1.1.1778542197` to wait for CI.
- [ ] Record the CI run URL in `tracker.md`.
- [ ] Do not continue if any release job is skipped, allowed to fail, or marked
  neutral while it owns an expected release artifact.

### T12.3 CI Green Criteria

- [ ] `preflight` proves Apple signing/notarization, Tauri signing if enabled,
  and manifest signing readiness.
- [ ] `build-assets` publishes both `arm64` and `x86_64` assets with kernel,
  initrd, rootfs, manifest, and manifest signature.
- [ ] `test` passes the full release CI suite.
- [ ] `build-app-macos` builds the `.app`, packages `.pkg`, verifies package
  payload manifest/signature, notarizes, staples, and uploads expected assets.
- [ ] `build-app-linux` builds expected `.deb` packages, verifies helper
  binaries and manifest/signature, and is release-blocking.
- [ ] `create-release` refuses to publish if expected assets, manifests,
  updater artifacts, or provenance are missing.

### T12.4 Live Release Asset Verification

- [ ] `gh release view v1.1.1778542197`.
- [ ] Download published `manifest.json` and `manifest.json.minisig`.
- [ ] Verify `manifest.json.minisig` with `config/manifest-sign.pub`.
- [ ] Download published `.pkg`; run `pkgutil --check-signature`,
  `spctl -a -vv -t install`, and `xcrun stapler validate`.
- [ ] Expand the published `.pkg` and verify signed manifest, both arch maps,
  helper binaries, app bundle, and package scripts.
- [ ] Download published `.deb` artifacts and verify signed manifest plus all
  eight host binaries, including `capsem-mcp-aggregator` and
  `capsem-mcp-builtin`.
- [ ] Run clean install proof from at least one downloaded package and record
  the command output path.

### T12.5 Release Landed Record

- [ ] Record final version, release URL, CI run URL, package asset names,
  manifest verification result, notarization/staple result, and clean-install
  proof in `tracker.md`.
- [ ] Confirm GitHub marks `v1.1.1778542197` as the latest release if intended.
- [ ] Confirm docs/release-page links point to the landed version.
- [ ] Mark this sprint complete only after CI is green and live assets verify.

## Proof Matrix

| Category | Required proof |
|---|---|
| Version | tag, changelog, binary metadata, release page, and release assets all say the exact `1.1.1778542197`. |
| CI | every required release job is green and release-blocking. |
| Artifacts | live manifest/signature, `.pkg`, `.deb`, and provenance assets are present and verified. |
| Install | at least one downloaded package cleanly installs and runs an installed CLI/VM smoke. |
| Release hygiene | immutable tag, release notes, latest-release metadata, and tracker evidence agree. |

## Landed Release Evidence

- [x] GitHub latest release is `v1.1.1778542197` with `.pkg`, amd64/arm64
  `.deb`, signed `manifest.json`, signed `manifest.json.minisig`, SBOM, and
  arm64/x86_64 boot assets.
- [x] Release workflow run `25703667428` completed successfully for
  `release: v1.1.1778542197`; the macOS job notarized, stapled, and validated
  the package, and `verify-release-downloads` exercised live GitHub downloads.
- [x] Main CI run `25723005949` and `Publish Site` run `25723006002` completed
  successfully for the site/download follow-up merge `d2a1de9`.
- [x] Live `capsem.org/install.sh` downloads `manifest.json` and
  `manifest.json.minisig`, verifies the manifest signature with minisign when
  available, checks package SHA256 values from the manifest, and installs
  `.pkg`/`.deb` artifacts with native installers.
- [x] Local release verification downloaded `manifest.json`,
  `manifest.json.minisig`, `Capsem-1.1.1778542197.pkg`,
  `Capsem_1.1.1778542197_amd64.deb`, and
  `Capsem_1.1.1778542197_arm64.deb`; manifest minisign verification and all
  package SHA256 checks matched the release metadata.
- [x] The hardened `scripts/verify_deb_payload.py` verifies both published
  `.deb` packages, including zstd payloads without embedded content-size
  headers.
- [x] Local macOS 26 package-assessment tools reported Code Signing subsystem
  errors for both the v1.0 and v1.1 release packages while CI macOS 14
  notarization/stapling succeeded; the follow-up release workflow now makes
  `pkgutil --check-signature` and `spctl -a -vv -t install` release-blocking
  immediately after stapling.

## Verification

- [ ] `git status --short`
- [ ] `git tag --list 'v1.1.*'`
- [ ] `git ls-remote --tags origin 'v1.1.*'`
- [ ] `just release v1.1.1778542197`
- [ ] `gh release view v1.1.1778542197`
- [ ] `gh release download v1.1.1778542197 --pattern manifest.json -D /tmp/capsem-v1.1.1778542197`
- [ ] `gh release download v1.1.1778542197 --pattern manifest.json.minisig -D /tmp/capsem-v1.1.1778542197`
- [ ] `minisign -Vm /tmp/capsem-v1.1.1778542197/manifest.json -p config/manifest-sign.pub`
- [ ] `gh release download v1.1.1778542197 --pattern '*.pkg' -D /tmp/capsem-v1.1.1778542197`
- [ ] `pkgutil --check-signature /tmp/capsem-v1.1.1778542197/Capsem-*.pkg`
- [ ] `spctl -a -vv -t install /tmp/capsem-v1.1.1778542197/Capsem-*.pkg`
- [ ] `xcrun stapler validate /tmp/capsem-v1.1.1778542197/Capsem-*.pkg`
- [ ] `pkgutil --expand-full /tmp/capsem-v1.1.1778542197/Capsem-*.pkg /tmp/capsem-v1.1.1778542197/pkg-expanded`
- [ ] `gh release download v1.1.1778542197 --pattern '*.deb' -D /tmp/capsem-v1.1.1778542197`
- [ ] `dpkg-deb --contents /tmp/capsem-v1.1.1778542197/*.deb | rg 'manifest\\.json(\\.minisig)?|capsem-mcp-(aggregator|builtin)'`

## Exit Criteria

- [ ] T11 was signed off before the tag was pushed.
- [ ] CI is green for `v1.1.1778542197`.
- [ ] Published assets are present, signed, notarized where applicable, and
  payload-verified.
- [ ] A downloaded package clean-install proof is recorded.
- [ ] The release is marked landed as `v1.1.1778542197` in `tracker.md`.
