# Release Policy Hardening Tracker

## Mission

Do not tag `v1.1.xxx` until the release artifacts, Policy V2 UI/runtime,
telemetry, docs, and package verification can prove the shipped behavior from a
clean install. This tracker is the execution board; each `T*.md` file is the
owning sub-sprint doc with detailed task lists.

## Execution Board

| Track | Status | Priority | Blocking Release? | Current focus |
|---|---:|---:|---:|---|
| T0 release artifacts and updater packaging | Not started | P0 | Yes | Signed manifests, package install truth, updater honesty. |
| T1 image builder and manifest compatibility | Not started | P1 | Yes | Manifest metadata, same-day versions, hard rootfs validation. |
| T2 frontend Policy V2 settings | Not started | P1 | Yes | Staged rules, rename/delete, mocks, import validation. |
| T3 policy hook runtime hardening | Not started | P1 | Yes | Loopback validation, body cap, fail-closed, MCP notification bypass. |
| T4 docs and release notes | Not started | P1 | Yes | No overclaims, no stale artifact/DNS docs. |
| T5 service/process/helper packaging | Not started | P0 | Yes | Linux helper binaries, env isolation, reload, route tests. |
| T6 telemetry/session tooling | Not started | P2 | Yes | Old DB compatibility, timeline layers, schema tests. |
| T7 swarm intake and review control | In progress | P2 | Yes | Swarm finding docs captured; expand implementation sub-sprints next. |
| T8 policy integration E2E | Not started | P1 | Yes | UI/config/service/runtime/telemetry proof path. |
| T9 release metadata and changelog | Not started | P1 | Yes | Version, changelog, latest release, and release page sync. |
| T10 focused verification | Not started | P0 | Yes | Per-track targeted proof before full suite. |
| T11 local release candidate gate | Not started | P0 | Yes | Preflight, full `just test`, `just install`, installed CLI/UI/full-launch sign-off. |
| T12 CI green release landing | Not started | P0 | Yes | Tag `v1.1.xxx`, wait for CI green, verify live release assets. |

## Active Swarm

- [x] None. Final investigation wave is captured in
  `sprints/release-policy-hardening/swarm.md` and `swarm-findings/`.

## Completed Swarm Intake

- [x] UI settings review: transferred to T2.
- [x] Release workflow review: transferred to T0/T5.
- [x] Image builder review: transferred to T1/T4.
- [x] Docs/release notes review: transferred to T4.
- [x] capsem-core policy hook review: transferred to T3.
- [x] service/process packaging review: transferred to T5.
- [x] logger/session review: transferred to T6.
- [x] CLI/update/install review: transferred to T0/T5.
- [x] app/updater shell review: transferred to T0.
- [x] MCP/guest packaging review: transferred to T5.
- [x] CI/package gate review: transferred to T0/T1/T5.
- [x] Sprint QA review: transferred to tracker/plan/T6/T7/T8.
- [x] Frontend Policy UI execution review: transferred to T2.
- [x] Policy integration E2E review: transferred to T2/T3/T5/T6/T8.
- [x] Policy hook/runtime security review: transferred to T3/T8.
- [x] Release artifact/package execution review: transferred to T0/T1/T5.
- [x] Docs/telemetry execution review: transferred to T4/T6.
- [x] Global sprint hygiene review: transferred to MASTER/T4/T7/tracker.
- [x] Second QA swarm: transferred to MASTER, plan, tracker, and T0-T8.
- [x] T9 release metadata/changelog review: transferred to T4/T7/T9.
- [x] T10/T11 verification review: transferred to T10/T11/tracker.
- [x] Package verification story review: transferred to T0/T1/T5/T10.
- [x] Tracker/doc consistency review after T9-T11: transferred to
  tracker/T7/T9/T10/T11.
- [x] Final UI policy/settings review: captured in
  `swarm-findings/ui-policy-settings.md`; transferred to T2/T8/T10, with
  runtime/image truth in T2.8/T8.6.
- [x] Final docs/release metadata review: captured in
  `swarm-findings/docs-release-metadata.md`; transferred to T4/T9/T11/T12.
- [x] Final sprint consistency review: captured in
  `swarm-findings/sprint-consistency.md`; pending T7/T10/T11 cleanup.
- [x] Final core policy/assets review: captured in
  `swarm-findings/core-policy-assets.md`; pending T1/T3/T8/T10 expansion.
- [x] Final service/process review: captured in
  `swarm-findings/service-process.md`; pending T3/T5/T8/T10 expansion.
- [x] Final CLI/install/updater review: captured in
  `swarm-findings/cli-updater-install.md`; pending T0/T5/T9/T10/T11
  expansion.
- [x] Final MCP policy boundary review: captured in
  `swarm-findings/mcp-policy-boundary.md`; pending T3/T5/T6/T8/T10
  expansion.
- [x] Final telemetry/session review: captured in
  `swarm-findings/telemetry-session.md`; pending T3/T6/T8/T10 expansion.
- [x] Final guest/image-builder review: captured in
  `swarm-findings/guest-image-builder.md`; pending T1/T5/T10 expansion.
- [x] Final CI packaging review: captured in
  `swarm-findings/ci-packaging.md`; pending T0/T1/T5/T10/T11 expansion.
- [x] Final verification-architecture review: captured in
  `swarm-findings/verification-architecture.md`; transferred to
  T2/T7/T8/T10/T11/T12.
- [x] Manual UI/CLI gate review: captured in
  `swarm-findings/manual-ui-cli-gates.md`; transferred to T10/T11.
- [x] CI release landing 1.1 review: captured in
  `swarm-findings/ci-release-landing-1-1.md`; transferred to T9/T11/T12.
- [x] Swarm transfer closeout review: captured in
  `swarm-findings/swarm-transfer-closeout-2026-05-10.md`; transferred to
  T2/T7/T8/T10/T12.

## Release Blockers

### P0

- [ ] `.pkg` must ship `manifest.json` and `manifest.json.minisig`; current CI
  signs the manifest after package construction.
- [ ] `.pkg` must ship a unified manifest with both `arm64` and `x86_64`; the
  current package path builds from `vm-assets-arm64`.
- [ ] `.deb` must seed or fetch signed `manifest.json` and
  `manifest.json.minisig`; current postinstall creates only the assets dir.
- [ ] `.deb` must include `capsem-mcp-aggregator` and `capsem-mcp-builtin`;
  current Linux package has the stale six-binary contract.
- [ ] Package install verification must start from package payloads, not a
  manually seeded manifest in `/tmp/capsem-home`.

### P1

- [ ] Release manifest mutation must preserve binary `min_assets`, `date`, and
  `deprecated`.
- [ ] Rootfs validation must be a hard pre-publish gate and must cover
  `capsem-dns-proxy` and `capsem-sysutil`.
- [ ] Tauri updater config/UI must be disabled or backed by real full-install
  updater artifacts; currently `latest.json` is configured but not published.
- [ ] Policy rule rename/type change must delete the old key and add the new
  key atomically in staged state.
- [ ] Staged/imported/generated policy rules must be visible before save.
- [ ] Hook endpoint validation must reject DNS lookalikes such as
  `127.evil.example`.
- [ ] Hook response body cap must be enforced while reading, not after buffering
  the whole response.
- [ ] Fail-closed configuration cannot silently become fail-open.
- [ ] Docs must not claim configured external hook dispatch if that path is not
  wired for this release.
- [ ] `config/policy-hook-openapi.json` must be tracked and verified in clean
  checkout CI.
- [ ] Policy hook controls must not be exposed as a shipped feature unless T8
  wires endpoint config and production dispatch.
- [ ] MCP notification frames must not bypass request policy/telemetry.
- [ ] Settings save must not hide reload failures for running VMs.

## Track Task Index

### T0 Release Artifacts and Updater Packaging

- [ ] T0.1 Decide final-vs-asset-snapshot package manifest contract.
- [ ] T0.2 MacOS Package Manifest and Signature.
- [ ] T0.3 Linux Package Manifest and Signature.
- [ ] T0.4 Verified Manifest Consumers.
- [ ] T0.5 Package Failure Semantics.
- [ ] T0.6 Desktop Updater Strategy.
- [ ] T0.7 Release Preflight and Post-Release Proof.

### T1 Image Builder and Manifest Compatibility

- [ ] T1.1 Preserve binary compatibility metadata during release manifest
  mutation.
- [ ] T1.2 Unify same-day asset version generation across full image builds and
  initrd repacks.
- [ ] T1.3 Safe Local Initrd Repack.
- [ ] T1.4 Asset Cleanup.
- [ ] T1.5 Rootfs Validation as a Hard Gate.
- [ ] T1.6 Documentation and Comments.

### T2 Frontend Policy Settings

- [ ] T2.1 Single Source for Settings Mocks.
- [ ] T2.2 Reviewable Pending Policy State.
- [ ] T2.3 Atomic Rename and Type Change.
- [ ] T2.4 Import and Draft Validation.
- [ ] T2.5 Generated Rule UX.
- [ ] T2.6 Runtime-Truthful Surfaces.
- [ ] T2.7 Component and Visual Coverage.
- [ ] T2.8 Runtime and Image Truth.

### T3 Policy Hook Runtime Hardening

- [ ] T3.1 Replace hostname-prefix loopback detection with exact localhost or
  parsed `IpAddr::is_loopback`.
- [ ] T3.2 Stream hook responses with a hard body cap.
- [ ] T3.3 Fail-Closed and Spec0 Semantics.
- [ ] T3.4 Audit Correctness.
- [ ] T3.5 MCP Notification Policy Bypass.
- [ ] T3.6 MCP Policy V2 Telemetry Semantics.
- [ ] T3.7 Bench Guardrails.

### T4 Docs and Release Notes

- [ ] T4.1 Release Claims.
- [ ] T4.2 Artifact and Install Docs.
- [ ] T4.3 Session Telemetry Docs.
- [ ] T4.4 Site and Benchmark Stale References.

### T5 Service, Process, and Helper Packaging

- [ ] T5.1 Add MCP helper binaries to Linux build, repack, postinst, simulate
  install, and tests.
- [ ] T5.2 Policy Spec Artifact and Routes.
- [ ] T5.3 Cleanup and Environment Isolation.
- [ ] T5.4 Rootfs Validation Coverage.
- [ ] T5.5 Runtime Config Reload Semantics.

### T6 Telemetry and Session Tooling

- [ ] T6.1 Make `policy_hook_events` optional/version-aware for old DBs in
  `check_session.py`.
- [ ] T6.2 Fix MCP/tool correlation SQL.
- [ ] T6.3 Timeline and Triage Coverage.
- [ ] T6.4 Frontend Session Tooling.
- [ ] T6.5 Schema Migration and Lifecycle Tests.

### T7 Swarm Intake

- [ ] T7.1 Intake Discipline.
- [ ] T7.2 Status Synchronization.
- [ ] T7.3 Swarm Depth Gate.

### T8 Policy Integration E2E

- [ ] T8.1 Decide Shipping Scope.
- [ ] T8.2 If Hook Dispatch Ships.
- [ ] T8.3 If Hook Dispatch Does Not Ship.
- [ ] T8.4 Running Session Apply Semantics.
- [ ] T8.5 Telemetry and Debug Surfaces.
- [ ] T8.6 Runtime Support Matrix.

### T9 Release Metadata and Changelog

- [ ] T9.1 Version Synchronization.
- [ ] T9.2 Changelog.
- [ ] T9.3 Latest Release Summary.
- [ ] T9.4 Release Page Metadata.
- [ ] T9.5 Commit Discipline.

### T10 Focused Verification

- [ ] T10.1 Package and Install Proof.
- [ ] T10.2 Asset and Manifest Proof.
- [ ] T10.3 Frontend Policy Proof.
- [ ] T10.4 Runtime Security Proof.
- [ ] T10.5 Service, Telemetry, and Integration Proof.
- [ ] T10.6 Docs and Metadata Proof.
- [ ] T10.7 Evidence Capture.
- [ ] T10.8 Evidence Ledger for Gate A/B proof, command transcripts, screenshot
  paths, console checks, package logs, owners, and release-blocking follow-ups.

### T11 Local Release Candidate Gate

- [ ] T11.1 Preflight.
- [ ] T11.2 Full Suite.
- [ ] T11.3 Local Package Generation and Install.
- [ ] T11.4 Elie + Codex Product Sign-Off.
- [ ] T11.5 Release Readiness Review.
- [ ] T11.6 Handoff to T12.

### T12 CI Green Release Landing

- [ ] T12.1 Pre-Tag Readiness.
- [ ] T12.2 Tag and CI Run.
- [ ] T12.3 CI Green Criteria.
- [ ] T12.4 Live Release Asset Verification.
- [ ] T12.5 Release Landed Record.

## Coverage Ledger

### T0

- Unit/contract: setup/update manifest verification tests.
- Functional: package payload assertions.
- Adversarial: tampered or missing manifest/signature fails loudly.
- E2E/VM: clean `.pkg`/`.deb` install plus setup/update/status.
- Telemetry: install/deferred state visible where applicable.
- Performance: n/a.
- Missing/deferred: macOS clean boot may remain manual if CI cannot run it.

### T1

- Unit/contract: manifest producer and asset manager tests.
- Functional: `_pack-initrd` preserves two arch maps.
- Adversarial: stale assets and legacy dirs cleanup.
- E2E/VM: rootfs validation in CI.
- Telemetry: n/a.
- Performance: no regression to asset resolution path.
- Missing/deferred: full VM boot covered by T0/T8.

### T2

- Unit/contract: settings model/import tests.
- Functional: component/store save/export/import tests.
- Adversarial: malformed imports rejected before staging.
- E2E/VM: visual Settings flow in app.
- Telemetry: n/a.
- Performance: n/a.
- Missing/deferred: browser screenshots required before sign-off.
- Runtime/image truth: asset health, image/fork UI, create defaults, and
  service/gateway status match shipped support.

### T3

- Unit/contract: hook URL/body/fallback/spec tests.
- Functional: hook client valid/invalid response tests.
- Adversarial: DNS lookalikes, streaming body, timeout, bad rewrite, MCP
  notification bypass.
- E2E/VM: optional service/hook fixture via T8.
- Telemetry: hook fallback rows distinguish failures and MCP action/mode is
  truthful.
- Performance: benchmark asserts matching rule path.
- Missing/deferred: configured external dispatch may be T8 debt.

### T4

- Unit/contract: n/a.
- Functional: docs/site builds.
- Adversarial: stale-term search.
- E2E/VM: n/a.
- Telemetry: docs include telemetry fields.
- Performance: benchmark docs updated.
- Missing/deferred: final wording waits for implementation decisions.

### T5

- Unit/contract: process/helper/env route tests.
- Functional: package helper discovery tests.
- Adversarial: env leak prevention.
- E2E/VM: install layout tests.
- Telemetry: n/a.
- Performance: cleanup avoids blocking runtime path.
- Missing/deferred: live service install tests may need harness update.

### T6

- Unit/contract: logger migration and script fixtures.
- Functional: `check_session.py` current/old DB runs.
- Adversarial: missing/new table compatibility.
- E2E/VM: session lifecycle/exhaustive tests.
- Telemetry: `capsem_timeline` dns/hook/audit/snapshot trace fixture.
- Performance: n/a.
- Missing/deferred: real-session trace proof can be T8 if fixture is
  insufficient.

### T7

- Unit/contract: n/a.
- Functional: every reviewer result transferred.
- Adversarial: stale status checks.
- E2E/VM: n/a.
- Telemetry: n/a.
- Performance: n/a.
- Missing/deferred: remains open until active swarm is drained.

### T8

- Unit/contract: settings/config/policy integration test.
- Functional: production-path policy behavior.
- Adversarial: unsupported surfaces hidden or rejected.
- E2E/VM: VM/service path if shipped.
- Telemetry: session/timeline assertion.
- Performance: n/a.
- Missing/deferred: if hook dispatch is not shipped, docs/UI must mark it debt.
- Runtime truth: T8.6 support matrix decides which UI runtime/image surfaces
  ship, and T2.8 proves or hides them.

### T9

- Unit/contract: version fields agree across Rust, Python, Tauri, and lockfile
  metadata.
- Functional: changelog, latest-release summary, and release page describe the
  implemented behavior.
- Adversarial: stale hook/updater/AppImage/DMG claims are searched and removed.
- E2E/VM: n/a.
- Telemetry: release notes mention telemetry only where T6/T8 prove it.
- Performance: n/a.
- Missing/deferred: waits for T0-T8 final implementation decisions.

### T10

- Unit/contract: targeted tests prove each changed logic boundary.
- Functional: package payloads, settings flows, reloads, docs builds, and
  metadata checks pass before full-suite cost.
- Adversarial: tampered manifests, malformed imports, hook bypasses, env leaks,
  and old DB compatibility are covered.
- E2E/VM: clean install, policy E2E or hidden-hook proof, Gate A JS UI proof,
  and Gate B CLI/VM proof are recorded.
- Telemetry: session DB/timeline checks prove policy and hook visibility.
- Performance: policy benchmark smoke and existing benchmark gates.
- Missing/deferred: any focused failure or missing Gate A/B evidence blocks
  T11 until it has an owner.

### T11

- Unit/contract: all focused gates from T10 are already green.
- Functional: `just doctor`, preflight, workflow checks, docs builds,
  `just test`, `just install`, installed CLI smoke, Gate C desktop launch, and
  Gate D installed app launch pass.
- Adversarial: no tag from dirty tree, no unstaged release metadata, no active
  unresolved swarm findings.
- E2E/VM: `just test-install`, installed `capsem doctor`, and clean
  `capsem-doctor` proof.
- Telemetry: final release notes match T6/T8 telemetry evidence.
- Performance: full benchmark gates inside `just test`.
- Missing/deferred: any P0/P1 gap or missing Elie sign-off keeps T12 on hold.

### T12

- Unit/contract: exact `1.1.xxx` version agrees across tag, metadata, and
  release docs.
- Functional: CI release workflow runs green and publishes expected assets.
- Adversarial: release job fails if expected packages, manifests, signatures,
  helper binaries, updater artifacts, or provenance are missing.
- E2E/VM: downloaded package clean-install proof is recorded.
- Telemetry: n/a.
- Performance: CI/full-suite benchmark gates are green.
- Missing/deferred: any live asset or CI failure reopens the owning track.

## Verification Commands

- [ ] `git diff --check`
- [ ] `pkgutil --expand-full /tmp/verify/Capsem-*.pkg /tmp/capsem-pkg-proof/pkg-expanded`
- [ ] `test -f /tmp/capsem-pkg-proof/pkg-expanded/**/*.app/Contents/Resources/manifest.json`
- [ ] `test -f /tmp/capsem-pkg-proof/pkg-expanded/**/*.app/Contents/Resources/manifest.json.minisig`
- [ ] `dpkg-deb -c /tmp/verify/capsem_*_*.deb | rg 'manifest\\.json|minisig|capsem-mcp-aggregator|capsem-mcp-builtin'`
- [ ] `cargo test -p capsem-core asset_manager -- --nocapture`
- [ ] `cargo test -p capsem-core policy_hook -- --nocapture`
- [ ] `cargo test -p capsem-core policy_hook_spec -- --nocapture`
- [ ] `cargo test -p capsem-core mcp_frame -- --nocapture`
- [ ] `cargo test -p capsem-service policy_hook -- --nocapture`
- [ ] `cargo test -p capsem-service cleanup -- --nocapture`
- [ ] `cargo test -p capsem-gateway all_non_root_paths_require_auth -- --nocapture`
- [ ] `cargo test -p capsem-app`
- [ ] `cargo test -p capsem-logger`
- [ ] `uv run pytest tests/test_repack_deb.py tests/capsem-install/test_installed_layout.py -q`
- [ ] `uv run pytest tests/test_docker.py::TestGenerateChecksums tests/test_gen_manifest.py tests/capsem-build-chain/test_manifest_regen.py -q`
- [ ] `uv run pytest tests/capsem-build-chain/test_create_hash_assets.py tests/capsem-install/test_asset_download.py tests/capsem-install/test_installed_layout.py -q`
- [ ] `uv run pytest tests/capsem-session-lifecycle/test_db_schema.py tests/capsem-session tests/capsem-session-exhaustive -q`
- [ ] `cd frontend && pnpm run check`
- [ ] `cd frontend && pnpm run test`
- [ ] `cd frontend && pnpm run build`
- [ ] `just dev-frontend`
- [ ] `just ui`
- [ ] Chrome DevTools MCP Settings visual/console proof for Policy add/edit/rename/delete/import/generated flows.
- [ ] `pnpm -C docs run build`
- [ ] `pnpm -C site run build`
- [ ] `uv run python3 scripts/extract-release-notes.py`
- [ ] `scripts/check-release-workflow.sh`
- [ ] `just test-install`
- [ ] `env UV_CACHE_DIR=/private/tmp/capsem-uv-cache just test`
- [ ] `just install`
- [ ] `~/.capsem/bin/capsem version`
- [ ] `~/.capsem/bin/capsem doctor`
- [ ] `~/.capsem/bin/capsem run "echo installed-cli-ok"`
- [ ] `just exec "capsem-doctor"`
- [ ] `just build-ui`
- [ ] `just run-ui --`
- [ ] `just release v1.1.xxx`
- [ ] `gh release view v1.1.xxx`
- [ ] `minisign -Vm /tmp/capsem-v1.1.xxx/manifest.json -p config/manifest-sign.pub`
- [ ] `pkgutil --check-signature /tmp/capsem-v1.1.xxx/Capsem-*.pkg`
- [ ] `spctl -a -vv -t install /tmp/capsem-v1.1.xxx/Capsem-*.pkg`
- [ ] `xcrun stapler validate /tmp/capsem-v1.1.xxx/Capsem-*.pkg`
- [ ] `dpkg-deb --contents /tmp/capsem-v1.1.xxx/*.deb | rg 'manifest\\.json(\\.minisig)?|capsem-mcp-(aggregator|builtin)'`

## Evidence Ledger

Use this ledger for T10/T11/T12 proof. Store screenshots, logs, transcripts,
package listings, and CI/live-release output in durable files and link the path
here. Do not rely on chat history as evidence.

| Gate | Status | Command / view | Expected proof | Evidence path | Owner | Follow-up / blocker |
|---|---|---|---|---|---|---|
| Gate A JS UI | Not run | `just dev-frontend` + browser Settings -> Policy | add/edit/rename/delete/import/generated/save/discard/reload-failure states; zero console errors/warnings | TBD | TBD | TBD |
| Gate B CLI/VM | Not run | `just exec "echo cli-ok"` + `just exec "capsem-doctor"` + T8 policy E2E | CLI output is understandable; VM doctor and policy/session proof pass | TBD | TBD | TBD |
| Gate C desktop dev launch | Not run | `just build-ui` + `just run-ui --` | embedded UI launches, stamped version visible, service/gateway reachable | TBD | TBD | TBD |
| Gate D installed package | Not run | `just install` + installed app + `~/.capsem/bin/capsem ...` | package-generated install works, installed CLI and installed desktop launch pass | TBD | TBD | TBD |
| T12 CI/live release | Not run | `just release v1.1.xxx` + `gh release view/download` | CI green, published assets signed/payload-verified, downloaded package proof recorded | TBD | TBD | TBD |

## Open Decisions

- [ ] Do we disable Tauri updater for this release, or build a full-install
  updater path that includes companion binaries and service/package state?
- [ ] Is configured external policy hook dispatch shipping in
  `1.1.xxx`, or is only Spec0/client/fail-closed/audit infrastructure
  shipping?
- [ ] Can clean macOS `.pkg` boot verification run in CI, or is it a manual
  release blocker with recorded proof?
- [ ] Should helper/external stdio MCP children inherit any parent env beyond
  explicitly configured server env?
- [ ] What exact `1.1.xxx` suffix ships, and does `_stamp-version` need to
  change before `just install` or `just cut-release`?

## Notes

- Do not tag or push until all P0/P1 blockers are fixed and verified.
- Do not mark a sub-sprint done unless its coverage ledger row is satisfied or
  an explicit missing/deferred item is recorded above.
- Keep `CHANGELOG.md`, `LATEST_RELEASE.md`, release page, and version files in
  sync after the fixes, not during exploratory review.
