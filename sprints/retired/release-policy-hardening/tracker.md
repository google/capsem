# Release Policy Hardening Tracker

## Mission

Do not tag `v1.1.1778542197` until the release artifacts, Policy V2 UI/runtime,
telemetry, docs, and package verification can prove the shipped behavior from a
clean install. This tracker is the execution board; each `T*.md` file is the
owning sub-sprint doc with detailed task lists.

## Execution Board

| Track | Status | Priority | Blocking Release? | Current focus |
|---|---:|---:|---:|---|
| T0 release artifacts and updater packaging | Implementation complete; live install proof captured in T11 | P0 | Yes | Signed package manifests, fail-loud setup, updater disabled, release workflow guards. |
| T1 image builder and manifest compatibility | Complete; focused rootfs proof passed in T10 | P1 | Yes | Numeric asset versions, all-arch repack manifests, per-arch cleanup, canonical hard rootfs validation. |
| T2 frontend Policy V2 settings | Implementation complete; Gate A live-service proof partially captured | P1 | Yes | Staged review, atomic rename/delete, generated mocks, import validation, reload failure banner/dismissal, and runtime/image truth are implemented; T10.3 captured Settings -> Policy generated/staged/discard visual proof under `just ui`, with exhaustive edit/rename/import/reload-failure behavior still covered by frontend tests. |
| T3 policy hook runtime hardening | Implementation complete; focused VM/E2E proof passed in T10 | P1 | Yes | Hook localhost validation, streaming body cap, fail-closed semantics, fallback audit rows, MCP notification denial, Policy V2 telemetry, and benchmark guards are implemented. |
| T4 docs and release notes | Implementation complete; final changelog/latest-release pass pending T9 | P1 | Yes | Hook overclaims, stale artifacts/updater text, telemetry docs, DNS wording, and benchmark gate docs are cleaned up. |
| T5 service/process/helper packaging | Implementation complete; focused package/VM proof passed in T10 | P0 | Yes | Helper binaries, route/spec proof, rootfs validation, env isolation, cleanup, and reload/refresh semantics are implemented; clean installed-package launch remains T11. |
| T6 telemetry/session tooling | Implementation complete; focused real-session trace proof passed in T10 | P2 | Yes | Old/core DB compatibility, current Policy V2 schema checks, MCP correlation, timeline/triage layers, frontend policy fields, lifecycle tests, and legacy migration coverage are implemented. |
| T7 swarm intake and review control | Owner mapping complete; downstream closeout open | P0 | Yes | FD01-FD14 transfer board, Galileo mapping audit, and command-validity sweep are captured; downstream blocker checkboxes stay open until T8-T13 resolves or defers each point. |
| T8 policy integration E2E | Implementation complete; focused VM proof passed in T10 | P1 | Yes | Hook dispatch deferred for 1.1, backend/frontend hook writes rejected, reload banner dismissal and live `/settings` reload/timeline E2E path implemented; focused live `/settings` + `/reload-config` MCP E2E passed on 2026-05-10. |
| T9 release metadata and changelog | Implementation complete; commit discipline pending | P1 | Yes | Exact `1.1.1778542197` stamp, changelog, latest release, release page, lockfile, stamp recipe, and internal dependency metadata are synchronized. |
| T10 focused verification | Complete; T11 blockers explicit | P0 | Yes | Focused Rust/Python/frontend/docs, strict `.deb` install, host doctor, T8 policy E2E, Gate A visual proof, Gate B command proof, rootfs validation, frontend coverage, and `.pkg` expansion/signature proof are green or captured; clean installed-package proof and full-suite gates remain T11. |
| T11 local release candidate gate | Full suite, private preflight, install smoke, installed doctor, demo UI, and final post-tray full gate green; manual sign-off open | P0 | Yes | Final `just test`, host doctor, `just exec "capsem-doctor"`, restored-private preflight, release workflow check, host package install, installed CLI run, installed doctor, rebuilt `.pkg` app-materialization fix, `/Applications` demo UI launch, `just run-ui --` process proof, and installed-app tray relaunch proof are captured. Elie Gate C/Gate D visual sign-off still blocks T12. |
| T12 CI green release landing | Release landed; CI hardening follow-up in progress | P0 | Yes | `v1.1.1778542197` is published/latest, release CI and site publish are green, live manifest/packages verify, and follow-up CI now blocks future releases on macOS pkg signature/Gatekeeper checks. |
| T13 kernel/netfilter recovery gate | Complete; full gate green on 2026-05-14 | P0 | Yes | Kernel/netfilter recovery now restores iptables tables and redirect installation, focused network-policy/session telemetry paths are passing, and local full `just test` completed green. |

## Active Swarm

- [x] No active swarm agents. T5 execution audit outputs from Descartes, Volta,
  and Hubble are captured in the finding docs and `swarm.md`.

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
  `swarm-findings/sprint-consistency.md`; FD03 pre-sprint subtask and owner
  rows added in T7/T10/T11.
- [x] Final core policy/assets review: captured in
  `swarm-findings/core-policy-assets.md`; FD04 pre-sprint subtask and owner
  rows added in T1/T3/T6/T8/T10.
- [x] Final service/process review: captured in
  `swarm-findings/service-process.md`; FD05 pre-sprint subtask and owner rows
  added in T3/T5/T8/T10.
- [x] Final CLI/install/updater review: captured in
  `swarm-findings/cli-updater-install.md`; FD06 pre-sprint subtask and owner
  rows added in T0/T5/T9/T10/T11.
- [x] Final MCP policy boundary review: captured in
  `swarm-findings/mcp-policy-boundary.md`; FD07 pre-sprint subtask and owner
  rows added in T3/T5/T6/T8/T10.
- [x] Final telemetry/session review: captured in
  `swarm-findings/telemetry-session.md`; FD08 pre-sprint subtask and owner
  rows added in T3/T6/T8/T10.
- [x] Final guest/image-builder review: captured in
  `swarm-findings/guest-image-builder.md`; FD09 pre-sprint subtask and owner
  rows added in T1/T5/T10.
- [x] Final CI packaging review: captured in
  `swarm-findings/ci-packaging.md`; FD10 pre-sprint subtask and owner rows
  added in T0/T1/T5/T10/T11/T12.
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
- [x] T7 transfer mapping audit: captured in
  `swarm-findings/swarm-transfer-closeout-2026-05-10.md`; no orphaned P0/P1
  findings found, stale timing/status wording corrected, and the missing
  conditional hook E2E test path assigned to T8.2.

## Release Blockers

### P0

- [x] `.pkg` must ship `manifest.json` and `manifest.json.minisig`; package
  construction now copies the signed asset snapshot and CI expands the payload.
- [x] `.pkg` must ship a unified manifest with both `arm64` and `x86_64`; the
  package path now builds from the unified asset map.
- [x] `.deb` must seed or fetch signed `manifest.json` and
  `manifest.json.minisig`; postinstall now copies signed manifests into the
  user asset layout.
- [x] `.deb` must include `capsem-mcp-aggregator` and `capsem-mcp-builtin`;
  the Linux package script now carries the full helper binary set.
- [x] Package install verification must start from package payloads, not a
  manually seeded manifest in `/tmp/capsem-home`; strict `.deb` install and
  fresh `.pkg` expansion/signature checks now prove package-seeded manifests.

### P1

- [x] Release manifest mutation must preserve binary `min_assets`, `date`, and
  `deprecated`.
- [x] Rootfs validation must be a hard pre-publish gate and must cover
  `capsem-dns-proxy` and `capsem-sysutil`.
- [x] Tauri updater config/UI must be disabled or backed by real full-install
  updater artifacts; unsupported updater config and frontend controls are now
  removed.
- [x] Policy rule rename/type change must delete the old key and add the new
  key atomically in staged state.
- [x] Staged/imported/generated policy rules must be visible before save.
- [x] Hook endpoint validation must reject DNS lookalikes such as
  `127.evil.example`.
- [x] Hook response body cap must be enforced while reading, not after buffering
  the whole response.
- [x] Fail-closed configuration cannot silently become fail-open.
- [x] Docs must not claim configured external hook dispatch if that path is not
  wired for this release.
- [x] `config/policy-hook-openapi.json` must be tracked and verified in clean
  checkout CI.
- [x] Policy hook controls must not be exposed as a shipped feature unless T8
  wires endpoint config and production dispatch.
- [x] MCP notification frames must not bypass request policy/telemetry in the
  framed runtime unit path; T8/T10 still own VM/E2E integration proof.
- [x] Settings save must not hide reload failures for running VMs.

## Track Task Index

### T0 Release Artifacts and Updater Packaging

- [x] T0.1 Decide final-vs-asset-snapshot package manifest contract.
- [x] T0.2 MacOS Package Manifest and Signature.
- [x] T0.3 Linux Package Manifest and Signature.
- [x] T0.4 Verified Manifest Consumers.
- [x] T0.5 Package Failure Semantics.
- [x] T0.6 Desktop Updater Strategy.
- [x] T0.7 Release Preflight and Post-Release Proof implementation; full clean
  package install proof remains tracked in T10/T11.

### T1 Image Builder and Manifest Compatibility

- [x] T1.1 Preserve binary compatibility metadata during release manifest
  mutation.
- [x] T1.2 Unify same-day asset version generation across full image builds and
  initrd repacks.
- [x] T1.3 Safe Local Initrd Repack.
- [x] T1.4 Asset Cleanup.
- [x] T1.5 Rootfs Validation as a Hard Gate.
- [x] T1.6 Documentation and Comments.

Verification recorded:
`cargo test -p capsem-core asset_manager -- --nocapture` (43 passed),
`cargo test -p capsem paths -- --nocapture` (15 passed), and
`env UV_CACHE_DIR=/private/tmp/capsem-uv-cache uv run pytest tests/test_docker.py::TestGenerateChecksums tests/test_gen_manifest.py tests/capsem-build-chain/test_manifest_regen.py tests/capsem-build-chain/test_create_hash_assets.py tests/test_release_workflow_policy.py tests/test_validate.py::TestE302 tests/capsem-rootfs-artifacts/test_rootfs_artifacts.py -q` (45 passed).

### T2 Frontend Policy Settings

- [x] T2.1 Single Source for Settings Mocks.
- [x] T2.2 Reviewable Pending Policy State.
- [x] T2.3 Atomic Rename and Type Change.
- [x] T2.4 Import and Draft Validation.
- [x] T2.5 Generated Rule UX.
- [x] T2.6 Runtime-Truthful Surfaces for current release UI; T8.6 still owns
  the final runtime support matrix.
- [x] T2.7 Component coverage and frontend verification; Gate A visual proof
  remains T10.3 because the local service was unavailable during the T2 smoke.
- [x] T2.8 Runtime and Image Truth for hidden/default release contract; T8 owns
  any later production image/fork selector decision.

Verification recorded:
`cd frontend && pnpm --config.store-dir=/Users/elie/Library/pnpm/store/v10 exec vitest run`
(17 files, 381 passed),
`pnpm -C frontend test -- src/lib/__tests__/settings-store.test.ts src/lib/__tests__/settings-page-reload-banner.test.ts src/lib/__tests__/api.test.ts src/lib/__tests__/settings-export.test.ts src/lib/models/__tests__/settings-model.test.ts src/lib/__tests__/policy-rules-section.test.ts`
(19 files, 388 tests passed),
`cd frontend && pnpm --config.store-dir=/Users/elie/Library/pnpm/store/v10 run check`
(0 errors/warnings),
`cd frontend && pnpm --config.store-dir=/Users/elie/Library/pnpm/store/v10 run build`
(2 pages built),
`env UV_CACHE_DIR=/private/tmp/capsem-uv-cache uv run pytest tests/test_config.py::TestGenerateDefaultsJsonConformance::test_mock_ts_not_stale -q`
(1 passed after sandbox escalation for `psutil.process_iter`), and
`git diff --check` (passed). New Tab asset-unknown browser smoke captured at
`/var/folders/l5/jg8zh4215ll399vd5mcp9sp40000gn/T/chrome-devtools-mcp-ZwCoIv/screenshot.png`.
Full Settings -> Policy visual proof remains T10.3/Gate A because the local
service was unavailable during the T2 smoke.

### T3 Policy Hook Runtime Hardening

- [x] T3.1 Replace hostname-prefix loopback detection with exact localhost or
  parsed `IpAddr::is_loopback`.
- [x] T3.2 Stream hook responses with a hard body cap.
- [x] T3.3 Fail-Closed and Spec0 Semantics.
- [x] T3.4 Audit Correctness.
- [x] T3.5 MCP Notification Policy Bypass.
- [x] T3.6 MCP Policy V2 Telemetry Semantics.
- [x] T3.7 Bench Guardrails.

Verification recorded:
`cargo test -p capsem-core policy_hook -- --nocapture` (23 passed; rerun
escalated because the raw TCP streaming test binds localhost),
`cargo test -p capsem-core policy_hook_spec -- --nocapture` (6 passed),
`cargo test -p capsem-core mcp_frame -- --nocapture` (50 passed),
`cargo test -p capsem-core mcp_endpoint -- --nocapture` (9 passed),
`cargo test -p capsem-logger mcp_call -- --nocapture` (15 passed), and
`cargo bench -p capsem-core --bench policy_v2 -- --sample-size 10 --warm-up-time 0.1 --measurement-time 0.2`
(completed; benchmark setup assertions passed). A parallel cargo test attempt
hit a codesign artifact lock; the affected suites passed sequentially.

### T4 Docs and Release Notes

- [x] T4.1 Release Claims.
- [x] T4.2 Artifact and Install Docs.
- [x] T4.3 Session Telemetry Docs.
- [x] T4.4 Site and Benchmark Stale References.

Verification recorded:
`rg -n "dnsmasq|vsock:?5003|DMG|\\.dmg|AppImage|image < 12MB|12MB" README.md docs/src/content/docs site/src`
(no matches),
`rg -n "latest\\.json" README.md docs/src/content/docs site/src` (no matches),
hook-overclaim telemetry scan (no matches),
`pnpm -C docs run build` (45 pages built), and
`pnpm -C site run build` (1 page built after installing missing site deps).

### T5 Service, Process, and Helper Packaging

- [x] T5.1 Add MCP helper binaries to Linux build, repack, postinst, simulate
  install, and tests.
- [x] T5.2 Policy Spec Artifact and Routes.
- [x] T5.3 Cleanup and Environment Isolation.
- [x] T5.4 Rootfs Validation Coverage.
- [x] T5.5 Runtime Config Reload Semantics.

Verification recorded:
`cargo test -p capsem-core checked_in_artifact_matches_rust_export -- --nocapture`
(1 passed),
`cargo test -p capsem-gateway all_non_root_paths_require_auth -- --nocapture`
(1 passed),
`cargo test -p capsem-service cleanup -- --nocapture` (3 passed),
`cargo test -p capsem-service mcp_refresh_surfaces_process_failure -- --nocapture`
(1 passed after sandbox escalation for temporary UDS bind),
`cargo test -p capsem-core stdio_child_base_env_allows_trace_and_execution_only -- --nocapture`
(1 passed),
`cargo test -p capsem-process aggregator_parent_env_allows_execution_and_logging_only -- --nocapture`
(1 passed),
`cargo test -p capsem-process mcp_runtime -- --nocapture` (4 passed),
`cargo test -p capsem-proto reload_config_result_roundtrip -- --nocapture`
(1 passed),
`cargo test -p capsem-mcp-aggregator -- --nocapture` (0 tests; compile passed),
`uv run pytest tests/test_package_scripts.py tests/test_repack_deb.py -q`
(3 passed, 6 skipped),
`uv run pytest tests/test_repack_deb.py tests/capsem-install/test_installed_layout.py -q`
(15 passed, 6 skipped),
`uv run pytest tests/capsem-install/test_installed_layout.py tests/capsem-install/test_smoke.py tests/capsem-install/test_reinstall.py -q`
(17 passed, 3 skipped),
`uv run pytest tests/test_release_workflow_policy.py tests/capsem-rootfs-artifacts/ -q`
(26 passed), and
`uv run pytest tests/capsem-gateway/test_gw_auth.py tests/capsem-gateway/test_gw_proxy.py -q`
(19 passed). `just cross-compile` and package payload expansion commands
remain T10/T11 because they require generated release artifacts.

### T6 Telemetry and Session Tooling

- [x] T6.1 Make `policy_hook_events` optional/version-aware for old DBs in
  `check_session.py`.
- [x] T6.2 Fix MCP/tool correlation SQL.
- [x] T6.3 Timeline and Triage Coverage.
- [x] T6.4 Frontend Session Tooling.
- [x] T6.5 Schema Migration and Lifecycle Tests.

Proof: `cargo test -p capsem-logger` (98 unit + 126 roundtrip passed),
`cargo test -p capsem-core policy_hook -- --nocapture` (23 passed after
localhost-bind escalation), `cargo test -p capsem-service timeline_ --
--nocapture` (5 passed), `cargo test -p capsem-service triage_ -- --nocapture`
(1 passed), `cargo test -p capsem-mcp timeline_tool_schema -- --nocapture`
(1 passed), `uv run pytest tests/capsem-session-lifecycle/test_db_exists.py
tests/capsem-session-lifecycle/test_db_schema.py -q` (13 passed),
`uv run pytest tests/capsem-session tests/capsem-session-exhaustive -q`
(52 passed, 1 skipped), `uv run pytest
tests/capsem-session/test_check_session_compat.py -q` (2 passed),
`pnpm -C frontend run check` (0 errors/warnings), frontend Vitest suite via
`pnpm -C frontend test -- src/lib/__tests__/sql-policy-fields.test.ts`
(18 files, 383 tests passed), and `git diff --check`.

### T7 Swarm Intake

- [x] T7.1 Intake Discipline.
- [x] T7.2 Status Synchronization.
- [x] T7.3 Swarm Depth Gate.
- [ ] T7.4 Final Closeout.

### T8 Policy Integration E2E

- [x] T8.1 Decide Shipping Scope.
- [x] T8.2 If Hook Dispatch Ships: deferred post-1.1 because configured
  external hook dispatch does not ship in `1.1.1778542197`.
- [x] T8.3 If Hook Dispatch Does Not Ship.
- [x] T8.4 Running Session Apply Semantics: implementation and UI tests done;
  focused VM proof passed in T10.
- [x] T8.5 Telemetry and Debug Surfaces: timeline assertion added to focused
  E2E path and passed in T10.
- [x] T8.6 Runtime Support Matrix.

### T9 Release Metadata and Changelog

- [x] T9.1 Version Synchronization.
- [x] T9.2 Changelog.
- [x] T9.3 Latest Release Summary.
- [x] T9.4 Release Page Metadata.
- [ ] T9.5 Commit Discipline.

### T10 Focused Verification

- [x] T10.1 Package and Install Proof.
- [x] T10.2 Asset and Manifest Proof.
- [x] T10.3 Frontend Policy Proof.
- [x] T10.4 Runtime Security Proof.
- [x] T10.5 Service, Telemetry, and Integration Proof.
- [x] T10.6 Docs and Metadata Proof.
- [x] T10.7 Evidence Capture.
- [x] T10.8 Evidence Ledger for Gate A/B proof, command transcripts, screenshot
  paths, console checks, package logs, owners, and release-blocking follow-ups.

### T11 Local Release Candidate Gate

- [x] T11.1 Preflight: local tooling checks, restored `private/` Apple
  signing/notarization checks, and passwordless manifest signing checks pass.
- [x] T11.2 Full Suite.
- [x] T11.3 Local Package Generation and Install smoke; installed doctor,
  canonical installed app location, and tray relaunch proof are captured.
- [ ] T11.4 Elie + Codex Product Sign-Off.
- [x] T11.5 Release Readiness Review.
- [x] T11.6 Handoff to T12 hold recorded; no tag/push/PR until Gate C/Gate D
  local sign-off is closed.

### T12 CI Green Release Landing

- [ ] T12.1 Pre-Tag Readiness.
- [ ] T12.2 Tag and CI Run.
- [ ] T12.3 CI Green Criteria.
- [ ] T12.4 Live Release Asset Verification.
- [ ] T12.5 Release Landed Record.

### T13 Kernel Netfilter Recovery Gate

- [x] T13.1 Failure Baseline.
- [x] T13.2 Deterministic Kernel Line.
- [x] T13.3 Build-Time Netfilter Contract.
- [x] T13.4 Boot-Time Fail Closed.
- [x] T13.5 Test Hardening.
- [x] T13.6 Rebuild + Focused Verification.
- [x] T13.7 Full Gate.
- [x] Evidence captured: local `just test` pass on 2026-05-14.

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

- Unit/contract: settings model/import tests and generated mock drift gate.
- Functional: component/store save/export/import tests, reload retry tests, and
  service-default create payload tests.
- Adversarial: malformed callback, decision, bucket mismatch, invalid rewrite,
  duplicate import keys, unsupported hook/dns controls, and missing/unknown
  asset states are rejected or hidden before staging/creation.
- E2E/VM: New Tab asset-unknown browser smoke captured; Settings -> Policy
  generated-rule and staged-rule/discard visual proof captured under `just ui`.
- Telemetry: n/a.
- Performance: n/a.
- Missing/deferred: full manual replay of rename/delete/import/save/reload
  failure remains open; frontend tests cover those behaviors.
- Runtime/image truth: asset health unknown is not ready, stale `/images` API is
  removed, image selection is hidden unless T8 ships it, create defaults are
  service-owned unless the user explicitly overrides.

### T3

- Unit/contract: hook URL/body/fallback/spec tests passed.
- Functional: hook client valid/invalid response tests passed.
- Adversarial: DNS lookalikes, streaming body, timeout, bad rewrite, and MCP
  notification bypass tests passed.
- E2E/VM: service/hook and MCP integration proof remains T8/T10.
- Telemetry: hook fallback rows distinguish failures and MCP action/mode is
  truthful in focused Rust/logger tests.
- Performance: policy benchmark asserts matching rule path before measuring.
- Missing/deferred: configured external dispatch and VM-level MCP policy proof
  remain T8/T10 debt.

### T4

- Unit/contract: n/a.
- Functional: docs/site builds passed.
- Adversarial: stale-term searches for old artifacts, updater feed, DNS
  wording, hook overclaims, and old benchmark gate passed.
- E2E/VM: n/a.
- Telemetry: session telemetry docs include `policy_hook_events`,
  `WriteOp::PolicyHookEvent`, Policy V2 action/mode fields, and normalized MCP
  `block` action wording.
- Performance: benchmark docs updated.
- Missing/deferred: T9 owns final curated `CHANGELOG.md`/`LATEST_RELEASE.md`
  after T0-T8 implementation decisions settle.

### T5

- Unit/contract: process/helper/env route tests.
- Functional: package helper discovery tests.
- Adversarial: env leak prevention.
- E2E/VM: install layout tests.
- Telemetry: n/a.
- Performance: cleanup avoids blocking runtime path.
- Missing/deferred: generated `.pkg`/`.deb` payload inspection and full
  running-VM reload/domain-policy E2E remain T10/T11 and T8/T10.

### T6

- Unit/contract: logger migration and script fixtures passed.
- Functional: `check_session.py` current/old DB runs passed.
- Adversarial: missing/new table compatibility and legacy timeline column
  fallback tests passed.
- E2E/VM: session lifecycle/exhaustive tests passed.
- Telemetry: `capsem_timeline` dns/hook/audit/snapshot trace fixture and
  NULL-trace retention test passed; triage fixture covers DNS/hook/audit
  failure surfaces.
- Performance: n/a.
- Missing/deferred: real-session product-path trace proof remains T8/T10.

### T7

- Unit/contract: n/a.
- Functional: every reviewer result is represented by a FD01-FD14 pre-sprint
  subtask plus owner rows in T0-T13.
- Adversarial: stale status checks.
- E2E/VM: n/a.
- Telemetry: n/a.
- Performance: n/a.
- Missing/deferred: remains open until every downstream blocker row is resolved
  or deliberately deferred with an owner.

### T8

- Unit/contract: settings/config/policy integration test passed.
- Functional: production-path policy behavior path updated for `/settings` +
  `/reload-config`; focused VM proof passed in T10.
- Adversarial: unsupported hook surfaces hidden and rejected.
- E2E/VM: non-hook Policy V2 VM/service proof path implemented; local run
  blocked by `uv` cache sandbox and escalation usage limit.
- Telemetry: session/timeline assertion added to focused E2E path.
- Performance: n/a.
- Missing/deferred: configured external hook dispatch and image/fork selector
  are deferred for `1.1.1778542197`; docs/UI mark or hide them.
- Runtime truth: T8.6 support matrix records shipped/deferred UI surfaces, and
  T2.8 proves or hides them.

### T9

- Unit/contract: version fields agree across Rust, Python, Tauri, lockfile, and
  workspace/internal path dependency metadata.
- Functional: changelog, latest-release summary, and release page describe the
  implemented behavior.
- Adversarial: stale hook/updater/AppImage/DMG claims are searched and removed.
- E2E/VM: n/a.
- Telemetry: release notes mention telemetry only where T6/T8 prove it.
- Performance: n/a.
- Missing/deferred: commit/staging remains pending because this thread is not
  creating a release commit before T10/T11 gates; restored `private/` material
  now lets local workflow preflight pass the signing-readiness checks.

### T10

- Unit/contract: targeted tests prove each changed logic boundary.
- Functional: package payloads, settings flows, reloads, docs builds, and
  metadata checks pass before full-suite cost.
- Adversarial: tampered manifests, malformed imports, hook bypasses, env leaks,
  old DB compatibility, and missing local manifest-signing tooling are covered.
- E2E/VM: clean install, policy E2E, Gate A JS UI proof,
  and Gate B CLI/VM proof are recorded.
- Telemetry: session DB/timeline checks prove policy and hook visibility.
- Performance: policy benchmark smoke and existing benchmark gates.
- Missing/deferred: clean macOS `.pkg` install proof was moved to T11 and is
  now captured; T10 remains the focused-gate evidence source.

### T11

- Unit/contract: all focused gates from T10 are green, plus T11 regressions for
  psutil leak-scan attr prefetch, local manifest-signing prerequisites, and
  Colima doctor status handling.
- Functional: `just test`, `just doctor`, direct B3SUMS/signature checks,
  restored-private release preflight, Docker/systemd install e2e, Linux
  release `.deb` validation, host `just install` package generation, installed
  service health, installed CLI version, installed doctor, installed VM run
  smoke, Gate C `just run-ui --` process proof, and demo UI launch passed.
- Adversarial: stale `.deb` artifacts, stale asset-current checksums, missing
  local dev signatures, macOS protected-process psutil failures, and Colima
  `pipefail` status false negatives are now covered. No tag was created from
  the dirty tree.
- E2E/VM: final `just exec "capsem-doctor"` passed, 308 passed and 4 skipped;
  final `just test` integration passed in-VM diagnostics and host session
  checks.
- Telemetry: final release notes match T6/T8 telemetry evidence.
- Performance: full benchmark gates inside `just test`.
- Missing/deferred: Gate C/Gate D Elie visual sign-off remains open. CI release
  secrets and local restored private credentials are not the blocker.

### T12

- Unit/contract: exact `1.1.1778542197` version agrees across tag, metadata, and
  release docs.
- Functional: CI release workflow runs green and publishes expected assets.
- Adversarial: release job fails if expected packages, manifests, signatures,
  helper binaries, updater artifacts, or provenance are missing.
- E2E/VM: downloaded package clean-install proof is recorded.
- Telemetry: n/a.
- Performance: CI/full-suite benchmark gates are green.
- Missing/deferred: any live asset or CI failure reopens the owning track.

### T13

- Unit/contract: kernel build asserts required netfilter/iptables symbols
  after `olddefconfig`; missing symbols fail build.
- Functional: guest boot installs redirect rules and exposes iptables tables.
- Adversarial: redirect install failure aborts boot with explicit netfilter
  mismatch messaging (no silent degraded mode).
- E2E/VM: focused failing network-policy/session telemetry suites pass after
  asset rebuild.
- Telemetry: `net_events` and policy evidence return for guest curl/model
  traffic.
- Performance: n/a.
- Missing/deferred: none; this track blocks "next sprint" scope until full
  `just test` is green.

## Verification Commands

- [x] `git diff --check`
- [ ] `just build-assets arm64`
- [ ] `just build-assets x86_64`
- [ ] `just _pack-initrd`
- [ ] `uv run pytest -q tests/capsem-session-lifecycle/test_exec_events.py::test_exec_curl_creates_net_event`
- [ ] `uv run pytest -q tests/capsem-guest/test_guest_network.py::TestGuestNetwork::test_iptables_redirect`
- [ ] `uv run pytest -q tests/capsem-e2e/test_model_policy_mitm.py::test_guest_model_request_policy_block_records_session_db_no_leak`
- [ ] `uv run pytest -q tests/capsem-e2e/test_policy_v2_http_dns_mitm.py::test_guest_http_policy_v2_block_and_header_strip_records_session_db`
- [ ] `uv run pytest -q tests/capsem-gateway/test_mitm_policy.py::test_mitm_policy_telemetry`
- [ ] `pkgutil --expand-full /tmp/verify/Capsem-*.pkg /tmp/capsem-pkg-proof/pkg-expanded`
- [ ] `test -f /tmp/capsem-pkg-proof/pkg-expanded/**/*.app/Contents/Resources/manifest.json`
- [ ] `test -f /tmp/capsem-pkg-proof/pkg-expanded/**/*.app/Contents/Resources/manifest.json.minisig`
- [ ] `dpkg-deb -c /tmp/verify/capsem_*_*.deb | rg 'manifest\\.json|minisig|capsem-mcp-aggregator|capsem-mcp-builtin'`
- [x] `cargo test -p capsem-core asset_manager -- --nocapture`
- [x] `cargo test -p capsem-core policy_hook -- --nocapture`
- [x] `cargo test -p capsem-core policy_hook_spec -- --nocapture`
- [x] `cargo test -p capsem-core mcp_frame -- --nocapture`
- [x] `cargo test -p capsem-core mcp_endpoint -- --nocapture`
- [x] `cargo test -p capsem-core batch_update_settings_json_rejects_invalid_policy_inputs_atomically -- --nocapture`
- [x] `cargo test -p capsem-logger mcp_call -- --nocapture`
- [x] `cargo test -p capsem-service reload_config_returns_structured_failed_session_state -- --nocapture`
  (sandbox run failed on UDS bind; escalated rerun passed)
- [x] `cargo test -p capsem-service policy_hook -- --nocapture`
- [x] `cargo test -p capsem-service cleanup -- --nocapture`
- [x] `cargo test -p capsem-gateway all_non_root_paths_require_auth -- --nocapture`
- [x] `cargo test -p capsem-app`
- [x] `cargo test -p capsem`
- [x] `cargo test -p capsem-logger`
- [x] `uv run pytest tests/test_repack_deb.py tests/capsem-install/test_installed_layout.py -q`
- [x] `uv run pytest tests/test_package_scripts.py -q`
- [x] `uv run pytest tests/capsem-install/test_asset_download.py -q`
- [x] `uv run pytest tests/test_release_workflow_policy.py::test_create_release_preserves_binary_metadata -q`
- [x] `uv run pytest tests/test_release_workflow_policy.py -q`
- [x] `uv run pytest tests/test_docker.py::TestGenerateChecksums tests/test_gen_manifest.py tests/capsem-build-chain/test_manifest_regen.py -q`
- [x] `uv run pytest tests/capsem-build-chain/test_create_hash_assets.py tests/capsem-install/test_asset_download.py tests/capsem-install/test_installed_layout.py -q`
- [x] `uv run pytest tests/capsem-session-lifecycle/test_db_exists.py tests/capsem-session-lifecycle/test_db_schema.py -q`
- [x] `uv run pytest tests/capsem-session tests/capsem-session-exhaustive -q`
- [x] `cd frontend && pnpm run check`
- [x] `cd frontend && pnpm run test`
- [x] `pnpm -C frontend exec vitest run --coverage`
- [x] `pnpm -C frontend test -- src/lib/__tests__/settings-store.test.ts src/lib/__tests__/settings-page-reload-banner.test.ts src/lib/__tests__/api.test.ts src/lib/__tests__/settings-export.test.ts src/lib/models/__tests__/settings-model.test.ts src/lib/__tests__/policy-rules-section.test.ts`
- [x] `cd frontend && pnpm run build`
- [x] `PYTHONPYCACHEPREFIX=/private/tmp/capsem-pycache python3 -m py_compile tests/capsem-e2e/test_framed_mcp_mitm.py`
- [x] `uv run pytest tests/capsem-e2e/test_framed_mcp_mitm.py::test_framed_guest_mcp_policy_reload_blocks_existing_connection -q`
- [x] `env UV_CACHE_DIR=target/uv-cache uv run pytest tests/capsem-e2e/test_framed_mcp_mitm.py::test_framed_guest_mcp_policy_reload_blocks_existing_connection -q`
  (T12/policy confidence rerun: 1 passed)
- [x] `env UV_CACHE_DIR=target/uv-cache uv run pytest tests/capsem-e2e/test_policy_v2_http_dns_mitm.py::test_guest_http_policy_v2_block_and_header_strip_records_session_db -q`
  (T12/policy confidence rerun: 1 passed)
- [x] `just dev-frontend`
  (reached the app after escalated pnpm refresh; standalone Settings showed the
  expected live-gateway dependency/service-unavailable state, so final visual
  proof used `just ui`)
- [x] `just ui`
- [ ] Chrome DevTools MCP Settings visual/console proof for Policy add/edit/rename/delete/import/generated flows.
  Captured generated-rule and staged-rule/discard proof; rename/delete/import
  and reload-failure remain covered by frontend tests rather than full manual
  browser replay.
- [x] `pnpm -C docs run build`
- [x] `pnpm -C site run build`
- [x] `env UV_CACHE_DIR=/private/tmp/capsem-uv-cache uv run python3 scripts/extract-release-notes.py`
- [x] `env UV_CACHE_DIR=/private/tmp/capsem-uv-cache uv lock --check`
- [x] `cargo metadata --no-deps --format-version 1`
- [x] Active-release old-stamp scan across metadata, lockfile,
  `LATEST_RELEASE.md`, and `docs/src/content/docs/releases/1-1.md`
- [x] Current 1.1 release-note stale hook/updater/artifact scan
- [x] `scripts/doctor-common.sh`
- [x] `just doctor`
  (42 passed, 0 skipped, 0 warnings; minisign, Docker daemon, and Colima pass)
- [x] `env UV_CACHE_DIR=target/uv-cache scripts/preflight.sh`
  (40 passed, 0 failed after `private/` restore)
- [x] `bash scripts/sync-dev-assets.sh assets assets && minisign -Vm assets/manifest.json -x assets/manifest.json.minisig -p assets/manifest-sign.dev.pub`
- [x] `scripts/check-release-workflow.sh`
  (13 passed, 0 failed; passwordless minisign key verifies with
  `config/manifest-sign.pub`)
- [x] `just test-install`
  (strict `.deb` install path passed: 33 passed, 31 skipped; no
  `apt-get install -f` retry path remains)
- [x] `env UV_CACHE_DIR=/private/tmp/capsem-uv-cache just test`
- [x] `just install`
- [x] `~/.capsem/bin/capsem version`
- [x] `~/.capsem/bin/capsem doctor`
- [x] `~/.capsem/bin/capsem run "echo installed-demo-ok"`
- [x] `just exec "echo cli-ok"`
- [x] `just exec "capsem-doctor"`
- [x] `just build-ui`
- [x] `just run-ui --`
- [x] `open /Applications/Capsem.app`
- [x] `gh release view v1.1.1778542197`
- [x] `gh run view 25703667428 --json status,conclusion,headSha,url,jobs`
- [x] `gh run view 25723005949 --json status,conclusion,headSha,url,jobs`
- [x] `gh run view 25723006002 --json status,conclusion,headSha,url,jobs`
- [x] `minisign -Vm /private/tmp/capsem-release-verify/manifest.json -x /private/tmp/capsem-release-verify/manifest.json.minisig -p config/manifest-sign.pub`
- [x] Package SHA256 checks for `Capsem-1.1.1778542197.pkg`,
  `Capsem_1.1.1778542197_amd64.deb`, and
  `Capsem_1.1.1778542197_arm64.deb` matched the release manifest.
- [x] `uv run --offline python scripts/verify_deb_payload.py /private/tmp/capsem-release-verify/Capsem_1.1.1778542197_amd64.deb /private/tmp/capsem-release-verify/Capsem_1.1.1778542197_arm64.deb --minisign-pubkey config/manifest-sign.pub`
- [ ] Local `pkgutil --check-signature`, `spctl -a -vv -t install`, and
  `xcrun stapler validate` reported Code Signing subsystem errors on macOS 26
  for both v1.0 and v1.1 packages; follow-up release CI now makes
  `pkgutil`/`spctl` package assessment release-blocking on macOS CI.

## Evidence Ledger

Use this ledger for T10/T11/T12 proof. Store screenshots, logs, transcripts,
package listings, and CI/live-release output in durable files and link the path
here. Do not rely on chat history as evidence.

| Gate | Status | Command / view | Expected proof | Evidence path | Owner | Follow-up / blocker |
|---|---|---|---|---|---|---|
| T11 full suite, preflight, and host doctor | Pass | `just test`; `just doctor`; restored-private preflight; direct B3SUMS/signature checks | Full suite, Docker install e2e, Linux package validation, final host doctor, Apple/notary/manifest preflight, and post-repack asset integrity/signature checks pass | `sprints/release-policy-hardening/evidence/T11-2026-05-10-full-release-gate.md` | T11.1/T11.2/T11.5 | Gate C/Gate D visual sign-off still blocks T12. |
| Gate A JS UI | Partial | `just dev-frontend`; `just ui`; Chrome Settings -> Policy staging/discard flow | Policy settings render against a live gateway, generated rules are visible, a disposable HTTP block rule can be staged and discarded, and browser-side warning/error capture stays empty after navigation/staging | `sprints/release-policy-hardening/evidence/T10-2026-05-10-minisign-install-proof.md`; `sprints/release-policy-hardening/evidence/T10-gate-a-policy-ui-just-ui.png`; `sprints/release-policy-hardening/evidence/T10-gate-a-policy-ui-staged-rule.png` | T2/T10.3 | Full manual replay of rename/delete/import/save/reload-failure remains open; model/store/component tests already cover those behaviors. |
| Gate B CLI/VM | Pass | `just exec "echo cli-ok"` + `just exec "capsem-doctor"` + T8 policy E2E | CLI output is understandable; VM doctor and policy/session proof pass | `sprints/release-policy-hardening/evidence/T10-2026-05-10-minisign-install-proof.md`; `sprints/release-policy-hardening/evidence/T11-2026-05-10-full-release-gate.md` | T10.5/T10.8/T11.4 | Final VM doctor rerun passed in T11; manual product sign-off still belongs to Gate C/Gate D. |
| Gate C desktop dev launch | Partial | `just build-ui` + `just run-ui --` + browser screenshot | embedded UI launches, service/gateway reachable, and demo chrome does not show the build timestamp | `sprints/release-policy-hardening/evidence/T11-2026-05-10-full-release-gate.md`; `sprints/release-policy-hardening/evidence/T11-gate-c-no-build-stamp.png` | T11.4 | `just build-ui` and `just run-ui --` process proof passed; no-build-stamp screenshot is captured. Elie visual sign-off remains open until he accepts the installed app path. |
| Gate D installed package | Partial | Fresh `.pkg` build + `just install` + installed service/CLI/UI smoke + Docker/systemd `.deb` install e2e inside `just test` | package payload contains signed manifest/signature/dev pubkey, every helper binary, and a postinstall `/Applications/Capsem.app` materialization fallback; Linux install e2e is green; host service responds; installed CLI VM run and installed doctor pass; demo UI process runs from `/Applications/Capsem.app`; killed tray is relaunched by the installed service when the app launches or remains open | `sprints/release-policy-hardening/evidence/T10-2026-05-10-minisign-install-proof.md`; `sprints/release-policy-hardening/evidence/T11-2026-05-10-full-release-gate.md`; `/private/tmp/capsem-pkg-1.1.1778445002-expanded` | T0/T10.1/T11.3 | Elie visual sign-off remains open before T12. |
| T12 CI/live release | Pass with local macOS package-assessment caveat | `gh run view 25703667428`; `gh run view 25723005949`; `gh run view 25723006002`; `gh release view/download`; `minisign -Vm`; hardened `scripts/verify_deb_payload.py` | Release CI green, site publish green, published assets signed and SHA256-verified, downloaded `.deb` package payloads verified | `sprints/release-policy-hardening/T12-ci-release-landing.md#landed-release-evidence`; `/private/tmp/capsem-release-verify` | T12 | Local macOS 26 `pkgutil`/`spctl`/`stapler` reported Code Signing subsystem errors for both v1.0 and v1.1 packages; follow-up release CI now adds macOS pkg signature/Gatekeeper gates. |

## Pre-Sprint Transfer Ledger

T7 owns this ledger. It confirms that every completed finding doc was read and
routed into durable owner rows. Downstream
checkboxes in T7 stay open until implementation resolves or defers each point.

| Finding doc | T7 subtask | Owner rows |
|---|---|---|
| `ui-policy-settings.md` | FD01 | T2, T8, T10 |
| `docs-release-metadata.md` | FD02 | T0, T4, T6, T9, T10, T11, T12 |
| `sprint-consistency.md` | FD03 | T7, T10, T11 |
| `core-policy-assets.md` | FD04 | T1, T3, T6, T8, T10 |
| `service-process.md` | FD05 | T3, T5, T8, T10 |
| `cli-updater-install.md` | FD06 | T0, T5, T9, T10, T11 |
| `mcp-policy-boundary.md` | FD07 | T3, T5, T6, T8, T10 |
| `telemetry-session.md` | FD08 | T3, T6, T8, T10 |
| `guest-image-builder.md` | FD09 | T1, T5, T10 |
| `ci-packaging.md` | FD10 | T0, T1, T5, T10, T11, T12 |
| `verification-architecture.md` | FD11 | T2, T7, T8, T10, T11, T12 |
| `manual-ui-cli-gates.md` | FD12 | T10, T11, tracker evidence ledger |
| `ci-release-landing-1-1.md` | FD13 | T1, T5, T9, T11, T12 |
| `swarm-transfer-closeout-2026-05-10.md` | FD14 | T2, T7, T8, T9, T10, T12 |

## Open Decisions

- [x] Do we disable Tauri updater for this release, or build a full-install
  updater path that includes companion binaries and service/package state?
- [x] Is configured external policy hook dispatch shipping in
  `1.1.1778542197`, or is only Spec0/client/fail-closed/audit infrastructure
  shipping? Decision: external dispatch is deferred; only Spec0/client/audit
  infrastructure plus non-hook Policy V2 enforcement ships.
- [ ] Can clean macOS `.pkg` boot verification run in CI, or is it a manual
  release blocker with recorded proof?
- [ ] Should helper/external stdio MCP children inherit any parent env beyond
  explicitly configured server env?
- [x] What exact `1.1.1778542197` suffix ships, and does `_stamp-version` need to
  change before `just install` or `just cut-release`? Decision:
  `1.1.1778542197`; `_stamp-version` now defaults to `1.1.<timestamp>` and
  accepts `CAPSEM_RELEASE_VERSION` for exact stamping.

## Notes

- Do not tag or push until all P0/P1 blockers are fixed and verified.
- Do not mark a sub-sprint done unless its coverage ledger row is satisfied or
  an explicit missing/deferred item is recorded above.
- Keep `CHANGELOG.md`, `LATEST_RELEASE.md`, release page, and version files in
  sync after the fixes, not during exploratory review.
