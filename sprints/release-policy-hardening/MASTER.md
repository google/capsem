# Release Policy Hardening Sprint

## Goal

Prepare the next `1.1.1778542197` release candidate by fixing the blocker-class issues
found in the post-sprint swarm review: release artifacts must install and boot
on fresh machines, Policy V2 settings must be usable from the UI, hook runtime
must fail closed safely, and docs must not overclaim shipped behavior.

T9 selected exact version `1.1.1778542197` and owns keeping the stamp recipe,
docs, binaries, and tag plan on that line.

## Status

| Track | Status | Priority | Dependencies | Proof / Test Count | Owner Notes |
|---|---:|---:|---|---|---|
| T0 release artifacts | Implementation complete; live install proof captured in T11 | P0 | T1 manifest contract, T5 helpers | 20 package/install/updater checks | Packages carry signed manifests, setup/update/status/service verify them, updater is disabled, and release workflow guards are hardened. Clean package install proof is captured in T11; release remains held on manual sign-off. |
| T1 image/manifest pipeline | Complete; focused rootfs proof passed in T10 | P1 | None | 45 Python checks + 58 Rust checks + 15 rootfs artifact checks | Numeric asset-version ordering, shared same-day patch generation, all-arch local repack manifests, per-arch cleanup, canonical rootfs validation, and docs/comments are updated. |
| T2 UI policy settings | Implementation complete; Gate A visual proof partially captured | P1 | T8 hook scope for final runtime matrix | 388 frontend tests + check/build + mock drift gate + asset-state browser smoke + T10 `just ui` screenshots | Staged policy review, atomic rename/type change, generated mocks, import validation, reload failure banner/dismissal, hidden unsupported hook/image surfaces, and service-default creation are implemented. T10.3 captured generated-rule and staged-rule/discard visual proof; exhaustive rename/delete/import/reload-failure behavior remains covered by frontend tests. |
| T3 policy hook runtime | Implementation complete; focused VM/E2E proof passed in T10 | P1 | T8 hook scope | 103 focused Rust/logger checks + policy benchmark + focused T8 VM E2E | Localhost validation, streaming body cap, fail-closed fallback, Spec0 semantics, fallback audit rows, MCP notification denial, Policy V2 telemetry naming, and benchmark guards are implemented. |
| T4 docs and release notes | Implementation complete; final changelog/latest-release pass pending T9 | P1 | T0/T8 final decisions | stale-term scans + docs build + site build | Docs now distinguish hook infrastructure from configured dispatch, describe `.pkg`/`.deb` and disabled updater truth, include hook/Policy V2 telemetry fields, and remove stale DNS/benchmark claims. |
| T5 service/process/package helpers | Implementation complete; focused package/VM proof passed in T10 | P0 | T0 package layout, T1 rootfs gate | Focused Rust/Python checks + compile gate + T10 Gate B/E2E/package proof | Helper packaging, spec route/auth proof, rootfs validation, env isolation, async cleanup, and builtin-aware reload/refresh are implemented; clean installed-package launch remains T11. |
| T6 telemetry/session tooling | Implementation complete; focused real-session trace proof passed in T10 | P2 | T3/T8 telemetry semantics | Logger/core/service/MCP/session/frontend gates passed + focused T8 timeline assertion | Old/core DB compatibility, Policy V2 schema checks, MCP/tool correlation, dns/hook/audit/snapshot timeline layers, triage, frontend policy fields, lifecycle tests, and legacy migration coverage are implemented. |
| T7 swarm intake and review control | Owner mapping complete; downstream closeout open | P0 | T8-T13 downstream resolution | FD01-FD14 transfer trackers + Galileo audit | Final investigation wave and mapping audit are captured; every finding doc is linked to owner T-track rows, while downstream blocker checkboxes stay open until resolved or deferred. |
| T8 policy integration E2E | Implementation complete; focused VM proof passed in T10 | P1 | T2/T3/T5/T6 | Hook defer decision + 388 frontend tests + focused Rust checks + focused VM E2E | Configured external hook dispatch is deferred for `1.1.1778542197`; non-hook Policy V2 settings/reload/timeline E2E path, reload banner dismissal, backend hook rejection, runtime support matrix, and live `/settings` + `/reload-config` MCP E2E proof are implemented. |
| T9 release metadata and changelog | Implementation complete; commit discipline pending | P1 | T0-T8 final decisions | version sync + latest-release extraction + release page + docs build + partial workflow check | Exact `1.1.1778542197` stamp, changelog, latest release, 1.1 release page, lockfile, stamp recipe, and internal dependency metadata are synchronized; workflow preflight still needs local signing prerequisites. |
| T10 focused verification | Complete; T11 blockers explicit | P0 | T0-T9 fixes | focused Rust/Python/frontend/docs + `.deb` install + `.pkg` expansion + Gate A/B + T8 E2E proof | Targeted checks, host doctor, strict `just test-install`, `just exec "echo cli-ok"`, `just exec "capsem-doctor"`, focused T8 policy E2E, rootfs validation, `.pkg` expansion/signature proof, frontend coverage, and `just ui` visual evidence are passing/captured; clean installed-package proof remains a T11/manual-host gate. |
| T11 local release candidate gate | Full suite/private preflight/install smoke/installed doctor/demo UI green; manual sign-off open | P0 | T10 green | Full `just test`, final doctor, restored-private preflight, VM doctor, Docker install e2e, host install smoke | Final `just test`, final host `just doctor`, direct B3SUMS/signature checks, `just exec "capsem-doctor"`, restored Apple/notary/manifest preflight, `just install`, installed CLI run, installed doctor, rebuilt `.pkg` app-materialization fix, `/Applications` demo UI launch, `just run-ui --` process proof, and installed-app tray relaunch proof passed by 2026-05-11. Remaining blockers are Elie Gate C/Gate D visual sign-off and the no-tag/no-push hold before T12. |
| T12 CI green release landing | Release landed; CI hardening follow-up in progress | P0 | T11 signed off | CI green + live asset verification + follow-up package gates | `v1.1.1778542197` is published/latest, CI and site publish are green, live manifest/packages verify, and follow-up CI now blocks future releases on macOS pkg signature/Gatekeeper checks. |
| T13 kernel/netfilter recovery gate | Complete; full gate green on 2026-05-14 | P0 | T10-T12 baseline + fresh asset rebuild | focused VM/network-policy/session tests + full `just test` | Kernel/netfilter recovery is in place: guest iptables tables and redirect rules are available again, focused policy/session telemetry paths pass, and local full `just test` is green. |

## Phases

- Foundation: T0, T1, T5.
- Policy surface: T2, T3, T8.
- Audit/docs: T4, T6.
- Pre-sprint intake: T7.
- Release control: T9, T10.
- Local ship gate: T11.
- CI and publish: T12.
- Post-release stabilization gate: T13.

## Just Recipes

- `just test-install`
- `just test`
- `just dev-frontend`
- `just ui`
- `just build-ui`
- `just run-ui --`
- `just install`
- `just exec "capsem-doctor"`

## Execution Spine

1. Keep T7 closeout active: no active swarm finding can remain only in a finding doc,
   and every completed finding doc must be represented by a FD01-FD14
   pre-sprint subtask plus owner rows in the relevant T0-T13 trackers.
2. Resolve T8 hook shipping scope before finalizing T2 UI choices or T4/T9
   release language.
3. Fix foundation tracks T0/T1/T5 before trusting any install or CI proof.
4. Fix Policy/UI/runtime/telemetry tracks T2/T3/T6/T8 with focused tests.
5. Finalize docs and release metadata in T4/T9 only after the shipped behavior
   is known.
6. Run T10 focused verification and record command output/evidence paths in
   `tracker.md`.
7. Run T11 locally: full suite, package generation, local install, and Elie +
   Codex CLI/UI/full-launch verification.
8. Only after T11 is signed off, run T12: tag `v1.1.1778542197`, wait for CI, verify
   published assets, and mark the release landed.
9. Before opening any new sprint scope, close T13 by proving redirect-path
   integrity and full `just test` green on the current branch.

## Elie + Codex Manual Gates

These are explicit stop points. The executor must pause, share the running app
or command output, and get Elie's sign-off in `tracker.md` before moving on.

### Gate A: JS UI Policy Flow

When T2 is ready and before T10 marks frontend proof complete:

- Run `just dev-frontend`.
- Open the JS UI in a browser and verify Settings -> Policy together.
- Exercise add, edit same key, rename, type change, delete, import, generated
  single-stage, stage-all, save, discard, and reload-failure banner states.
- Confirm browser console has zero errors/warnings after interaction.
- Record screenshot paths or Chrome DevTools evidence in `tracker.md`.

### Gate B: CLI and VM Behavior

When T8 production-path proof is ready and before T10 closes:

- Run `just exec "echo cli-ok"`.
- Run `just exec "capsem-doctor"`.
- Run the chosen Policy V2 E2E command from T8 and inspect the session/timeline
  proof together.
- Confirm the CLI output is understandable and failures are loud/actionable.

### Gate C: Desktop Full Launch

When frontend changes are embedded and before T11 starts:

- Run `just build-ui`.
- Run `just run-ui --`.
- Verify the desktop shell launches, shows the stamped build/version, can reach
  the gateway/service, renders Settings -> Policy correctly, and reports any
  reload failure truthfully.

### Gate D: Local Package Install Final Gate

This is the final local gate before CI/tagging:

- Run `just install` to build the release package and install it locally.
- Verify the generated `.pkg` or `.deb` payload contains the signed manifest,
  all helper binaries, and the expected app/service files.
- From the installed layout, run `~/.capsem/bin/capsem version`,
  `~/.capsem/bin/capsem doctor`, and
  `~/.capsem/bin/capsem run "echo installed-cli-ok"`.
- Launch the installed desktop app and verify the same full-launch path as
  Gate C, this time from the package-installed app rather than the dev binary.
- No T12 CI release work starts until Elie signs off this gate in `tracker.md`.

## CI Change Checklist

All CI changes must be listed here and then implemented in the owning track.
The release workflow must fail before publish if any expected item is missing.

- `.github/workflows/release.yaml`: build both arch VM assets before macOS
  package construction and package a unified two-arch manifest.
- `.github/workflows/release.yaml`: sign and verify the package payload
  manifest with the manifest signing key that matches `config/manifest-sign.pub`.
- `.github/workflows/release.yaml`: expand the generated `.pkg` and `.deb` in
  CI, assert `manifest.json` plus `manifest.json.minisig`, and assert both
  `arm64` and `x86_64` manifest maps.
- `.github/workflows/release.yaml`: require `capsem-mcp-aggregator` and
  `capsem-mcp-builtin` in every published package layout.
- `.github/workflows/release.yaml`: make Linux package and rootfs validation
  release-blocking; remove or neutralize `continue-on-error` paths that can
  publish while expected packages or rootfs checks failed.
- `.github/workflows/release.yaml`: validate every guest binary required by
  `capsem-init`, including `capsem-dns-proxy` and `capsem-sysutil`.
- `.github/workflows/release.yaml`: preserve manifest binary metadata such as
  `date`, `deprecated`, and `min_assets` when adding release file metadata.
- `.github/workflows/release.yaml`: either disable Tauri updater expectations
  or publish and verify the exact `latest.json` and updater archives required
  by the configured updater path.
- `.github/workflows/release.yaml`: post-release verification must start from
  published package payloads and an empty install home, not a manually seeded
  manifest.
- `.github/workflows/release.yaml`: include kernel, initrd, rootfs, manifest,
  manifest signature, `.pkg`, and `.deb` artifacts in provenance/attestation
  where GitHub Actions supports it.
- `scripts/preflight.sh` and `scripts/check-release-workflow.sh`: verify Apple
  signing/notarization readiness, Tauri signing readiness if updater remains
  enabled, and manifest-signing readiness.
- Release job: after tag `v1.1.1778542197`, wait for CI green and require live
  GitHub release asset verification before declaring the release landed.

## Immediate Release Blockers

- macOS `.pkg` bundles `manifest.json` without `manifest.json.minisig`, while
  release boot hard-fails unsigned manifests.
- macOS `.pkg` builds from an arm64-only manifest; x86_64 macOS installs would
  not have an x86_64 asset entry.
- Linux `.deb` installs seed no `manifest.json` or `manifest.json.minisig`,
  while `capsem setup` still marks install complete after skipping asset check.
- Post-release verification checks published asset URLs and optional Linux deb
  update, but does not inspect the `.pkg` payload or boot a clean macOS install.
- Post-release verification can mask package defects by seeding the manifest
  manually before `capsem update --assets` instead of starting from an installed
  package layout.
- Linux `.deb` omits `capsem-mcp-aggregator` and `capsem-mcp-builtin`, which
  `capsem-process` expects next to itself.
- Tauri updater config points at `latest.json` and checks on launch, but the
  release workflow does not publish `latest.json` or compatible updater
  archives; the current updater path would also update only the app bundle, not
  companion binaries.
- `config/policy-hook-openapi.json` is now tracked and clean-checkout/static
  release tests parse it; T10/T11 still own final release artifact proof.
- Policy hook release scope is still undecided: T2 hides unsupported hook UI,
  but T8 must decide and prove whether configured external hook dispatch ships.
- MCP notification bypass is closed in the T3 framed runtime unit path; T8/T10
  still own VM/E2E proof for the integration path.

## Swarm Inputs Captured

- UI settings review: `PolicyRulesSection.svelte`, settings model/store/tests,
  mock settings drift.
- Release workflow review: `.github/workflows/release.yaml`, package scripts,
  manifest signing, post-release verification.
- Image builder review: `generate_checksums`, `_pack-initrd`, manifest v2,
  cleanup, stale docs.
- capsem-core review: policy hook runtime, Spec0 strictness, benchmark guards.
- Docs review: release page, security/session telemetry docs, public site stale
  references.
- Service/process review: helper packaging, env forwarding, session cleanup,
  route/auth coverage.
- Logger/session DB: check-session compatibility, MCP correlation SQL,
  timeline dns/hook/audit/snapshot coverage, migration fixture tests.
- CLI/update/install: fresh `.deb` manifest seeding, verified setup/update
  manifest loading, postinstall failure behavior.
- App/updater shell: missing updater assets, incompatible Tauri updater model,
  disconnected update UI/settings, stale About version.
- MCP/guest packaging: Linux helper omission, stale install tests, external
  stdio env inheritance, missing DNS proxy rootfs validation.
- Sprint QA: added T8 for policy integration E2E, split invalid multi-filter
  cargo commands, added coverage-ledger requirements, and captured the second
  QA swarm over the expanded docs.
- Final UI/docs/tracker wave: captured in `swarm-findings/` for T2/T4/T7/T8/T9
  and release metadata.
- Final core/service/CLI/MCP wave: captured in `swarm-findings/` for T0/T1/T3
  /T5/T6/T8/T10/T11 and helper/updater/runtime policy blockers.
- Final telemetry/guest/CI/verification wave: captured in `swarm-findings/`
  for session tooling, rootfs/image-builder, release packaging, and proposed
  sub-sprint splits.
- Pre-sprint transfer board: FD01-FD14 in `T7-active-review-followups.md`
  links every finding doc to owner rows in T0-T13 and keeps downstream
  blocker checkboxes open until implementation resolves them.

## Swarm Process

The swarm was run as a no-edit investigation pass, with an additional T7
mapping audit after T6 implementation. The control board is
`sprints/release-policy-hardening/swarm.md`; it is the first file to read after
compaction or handoff. Each agent owned one domain, returned severity-ranked
findings, and had its output copied into a durable finding doc under
`sprints/release-policy-hardening/swarm-findings/`.

### Resume Order

1. Read `swarm.md`.
2. Confirm the Finding Docs Index has no `In progress` rows.
3. Read each completed finding doc before continuing T8-T13 work.
4. Deduplicate overlapping P0/P1 findings.
5. Keep implementation sub-sprints expanded so every P0/P1 has exact files, tests,
   package/UI/docs/VM proof, and a release-gate owner.
6. Keep T0-T13 execution tasks synchronized with any newly captured audit
   result before moving to the next track.

### Completed Finding Docs

| Domain | Finding doc | Primary tracks |
|---|---|---|
| UI policy/settings support | `swarm-findings/ui-policy-settings.md` | T2, T8, T10 |
| Docs and release metadata | `swarm-findings/docs-release-metadata.md` | T4, T9, T11 |
| Sprint consistency | `swarm-findings/sprint-consistency.md` | T7, T10, T11 |
| Core policy/assets | `swarm-findings/core-policy-assets.md` | T1, T3, T8, T10 |
| Service/process integration | `swarm-findings/service-process.md` | T3, T5, T8, T10 |
| CLI/install/updater | `swarm-findings/cli-updater-install.md` | T0, T5, T9, T10, T11 |
| MCP policy boundary | `swarm-findings/mcp-policy-boundary.md` | T3, T5, T6, T8, T10 |
| Telemetry/session tooling | `swarm-findings/telemetry-session.md` | T3, T6, T8, T10 |
| Guest/image builder/rootfs | `swarm-findings/guest-image-builder.md` | T1, T5, T10 |
| CI packaging/release artifacts | `swarm-findings/ci-packaging.md` | T0, T1, T5, T10, T11 |
| Verification architecture | `swarm-findings/verification-architecture.md` | T2, T7, T8, T10, T11, T12 |
| Manual UI/CLI gates | `swarm-findings/manual-ui-cli-gates.md` | T10, T11 |
| CI release landing 1.1 | `swarm-findings/ci-release-landing-1-1.md` | T9, T11, T12 |
| Swarm transfer closeout | `swarm-findings/swarm-transfer-closeout-2026-05-10.md` | T2, T7, T8, T9, T10, T12 |
| T7 transfer mapping audit | `swarm-findings/swarm-transfer-closeout-2026-05-10.md` | T7, T8, tracker |

### Required Closeout Before Release Gates

- Keep repeated package-manifest, helper-binary, hook-scope, telemetry, and
  updater findings deduplicated across finding docs.
- Keep the proposed splits from `verification-architecture.md` and the final
  targeted swarm docs, especially T7.4 swarm closeout, T8 scope branches,
  T10.8 evidence ledger, T11.6 local handoff, T12 CI release landing, and a
  frontend runtime/image-truth track.
- Keep the FD01-FD14 pre-sprint subtasks in T7 synchronized with the
  `## Swarm Transfer Tracker` rows in T0-T13.
- Normalize invalid or future test commands before putting them in final
  verification gates.
- Keep the release hold active until T10 focused verification, T11 local
  release candidate gate, T12 CI release landing, and T13 post-release
  kernel/netfilter recovery gate are green.

## Completion Criteria

- Fresh macOS `.pkg` extraction proves bundled manifest includes both arch maps
  and a valid `.minisig` verified by `config/manifest-sign.pub`.
- Fresh Linux `.deb` install seeds a signed manifest or fails setup loudly until
  a signed manifest can be fetched.
- Fresh macOS install plus `capsem run capsem-doctor` is tested or explicitly
  gated in CI/manual release checklist.
- Linux `.deb` contents include all host helper binaries used by process/MCP.
- Desktop update behavior is release-honest: either Tauri updater is disabled
  until a full-package update path exists, or CI publishes compatible updater
  metadata/artifacts and verifies them.
- Release rootfs validation checks all guest binaries that `capsem-init`
  requires, including `capsem-dns-proxy` and `capsem-sysutil`.
- Rootfs validation is a hard pre-publish gate rather than a check hidden in a
  `continue-on-error` package job.
- Published manifest keeps binary `min_assets` metadata when adding file
  metadata and remains backward compatible with previous binaries.
- Policy settings UI shows staged new/imported/generated rules before save and
  renames delete the old rule key atomically.
- Settings save reports reload failures for running sessions instead of
  silently showing stale policy as applied.
- Hook runtime rejects DNS lookalike loopback names, enforces body cap while
  reading, and cannot silently configure fail-open under a fail-closed name.
- MCP notification frames cannot bypass Policy V2 enforcement or telemetry.
- Old session DBs inspect cleanly without false-red hook-table errors, and hook
  events plus DNS/audit/snapshot layers appear in timeline/trace views.
- Docs/release notes distinguish shipped hook Spec0/runtime infrastructure from
  not-yet-wired user/corp hook dispatch.
- T8 records that configured external hook dispatch does not ship in
  `1.1.1778542197`; UI/docs/tests reject or describe hook dispatch as
  infrastructure-only.
- Frontend runtime/image truth has an owner: asset readiness, image/fork UI
  contract, create defaults, and service/gateway status truth cannot remain
  only in swarm findings.
- Release metadata and version files are synchronized after fixes land.
- T10 focused verification is green before T11 local release candidate gate
  begins.
- Focused tests listed in each track pass, then `just test` is rerun before
  release tagging.
- `just install` generates and installs the package locally before CI/tagging,
  and Elie signs off CLI, JS UI, desktop launch, and installed app behavior.
- T12 waits for CI green and verifies live `v1.1.1778542197` release assets before the
  release is marked landed.
