# Release Policy Hardening Sprint

## Goal

Prepare the next `1.1.xxx` release candidate by fixing the blocker-class issues
found in the post-sprint swarm review: release artifacts must install and boot
on fresh machines, Policy V2 settings must be usable from the UI, hook runtime
must fail closed safely, and docs must not overclaim shipped behavior.

T9 owns the exact `1.1.xxx` version choice and any stamping-recipe change needed
to stop the old `1.0.{timestamp}` release line from leaking into docs, binaries,
or tags. Until T9 records the exact version, all user-facing release copy should
say `1.1.xxx`.

## Status

| Track | Status | Priority | Dependencies | Proof / Test Count | Owner Notes |
|---|---:|---:|---|---|---|
| T0 release artifacts | Not started | P0 | T1 manifest contract, T5 helpers | 10 package/install checks | Fresh install can fail because packages ship missing/unsigned manifests; updater artifacts are not published. |
| T1 image/manifest pipeline | Not started | P1 | None | 4 manifest/rootfs checks | Manifest compatibility and same-day asset version drift. |
| T2 UI policy settings | Not started | P1 | T8 hook scope | 7 frontend checks | UI can save policy rules, but rename/staged-review/mock data/reload errors are broken. |
| T3 policy hook runtime | Not started | P1 | T8 hook scope | 4 Rust/security checks | Localhost validation/body cap/fallback/MCP notification semantics need hardening. |
| T4 docs and release notes | Not started | P1 | T0/T8 final decisions | 4 docs/release checks | Docs overclaim hooks and still mention stale artifact formats. |
| T5 service/process/package helpers | Not started | P0 | T0 package layout, T1 rootfs gate | 8 package/service checks | Linux package omits MCP helper binaries; cleanup/env/rootfs/reload gaps. |
| T6 telemetry/session tooling | Not started | P2 | T3/T8 telemetry semantics | 6 session/timeline checks | Old-DB compatibility, timeline layers, triage, schema tests. |
| T7 swarm intake and review control | In progress | P2 | None active | swarm finding docs captured | Final investigation wave captured; expand implementation sub-sprints next. |
| T8 policy integration E2E | Not started | P1 | T2/T3/T5/T6 | 4 E2E/scope checks | Decide hook shipping scope and prove UI/config/runtime/telemetry path. |
| T9 release metadata and changelog | Not started | P1 | T0-T8 final decisions | 4 metadata checks | Version, changelog, latest release, and release page sync. |
| T10 focused verification | Not started | P0 | T0-T9 fixes | 20 targeted checks | Per-track proof before full-suite cost. |
| T11 local release candidate gate | Not started | P0 | T10 green | 12 local gates | Preflight, full `just test`, `just install`, installed CLI/UI/full-launch sign-off. |
| T12 CI green release landing | Not started | P0 | T11 signed off | 10 CI/live-release gates | Tag `v1.1.xxx`, CI green, release assets verified, release landed. |

## Phases

- Foundation: T0, T1, T5.
- Policy surface: T2, T3, T8.
- Audit/docs: T4, T6.
- Release control: T7, T9, T10.
- Local ship gate: T11.
- CI and publish: T12.

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

1. Close T7 first: no active swarm finding can remain only in a finding doc.
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
8. Only after T11 is signed off, run T12: tag `v1.1.xxx`, wait for CI, verify
   published assets, and mark the release landed.

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
- Release job: after tag `v1.1.xxx`, wait for CI green and require live
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
- `config/policy-hook-openapi.json` is referenced by `include_str!` tests and
  must be explicitly tracked/staged for clean CI.
- Policy hook controls are exposed in UI/config, but no production path loads
  hook endpoints or calls `PolicyHookClient`.
- MCP notification frames can bypass request policy/telemetry if non-notify
  methods dispatch before policy enforcement.

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

## Swarm Process

The swarm was run as a no-edit investigation pass before implementation. The
control board is `sprints/release-policy-hardening/swarm.md`; it is the first
file to read after compaction or handoff. Each agent owned one domain, returned
severity-ranked findings, and had its output copied into a durable finding doc
under `sprints/release-policy-hardening/swarm-findings/`.

### Resume Order

1. Read `swarm.md`.
2. Confirm the Finding Docs Index has no `In progress` rows.
3. Read each completed finding doc before editing T0-T12.
4. Deduplicate overlapping P0/P1 findings.
5. Expand implementation sub-sprints so every P0/P1 has exact files, tests,
   package/UI/docs/VM proof, and a release-gate owner.
6. Only then update T0-T12 from pending finding intake to execution tasks.

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
| Verification architecture | `swarm-findings/verification-architecture.md` | T7, T10, T11 |
| Manual UI/CLI gates | `swarm-findings/manual-ui-cli-gates.md` | T10, T11 |
| CI release landing 1.1 | `swarm-findings/ci-release-landing-1-1.md` | T9, T11, T12 |
| Swarm transfer closeout | `swarm-findings/swarm-transfer-closeout-2026-05-10.md` | T7, T10, T12 |

### Required Closeout Before Implementation

- Deduplicate repeated package-manifest, helper-binary, hook-scope, telemetry,
  and updater findings across finding docs.
- Add the proposed splits from `verification-architecture.md` and the final
  targeted swarm docs, especially T7.4 swarm closeout, T8 scope branches,
  T10.8 evidence ledger, T11.6 local handoff, T12 CI release landing, and a
  frontend runtime/image-truth track.
- Normalize invalid or future test commands before putting them in final
  verification gates.
- Keep the release hold active until T10 focused verification, T11 local
  release candidate gate, and T12 CI release landing are green.

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
- T8 records whether configured external hook dispatch ships; UI/docs/tests
  match that decision.
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
- T12 waits for CI green and verifies live `v1.1.xxx` release assets before the
  release is marked landed.
