# T7: Swarm Intake and Review Control

## Objective

Keep swarm review intake explicit. Every active agent result must either be
transferred into T0-T12 with tasks/tests or kept here as an unresolved question
with an owner and next action.

## Owned Files

- `sprints/release-policy-hardening/MASTER.md`
- `sprints/release-policy-hardening/plan.md`
- `sprints/release-policy-hardening/tracker.md`
- `sprints/release-policy-hardening/T0-release-artifacts.md`
- `sprints/release-policy-hardening/T1-image-manifest-pipeline.md`
- `sprints/release-policy-hardening/T2-frontend-policy-settings.md`
- `sprints/release-policy-hardening/T3-policy-hook-runtime.md`
- `sprints/release-policy-hardening/T4-docs-release-notes.md`
- `sprints/release-policy-hardening/T5-service-process-packaging.md`
- `sprints/release-policy-hardening/T6-telemetry-session-tooling.md`
- `sprints/release-policy-hardening/T7-active-review-followups.md`
- `sprints/release-policy-hardening/T8-policy-integration-e2e.md`
- `sprints/release-policy-hardening/T9-release-metadata-changelog.md`
- `sprints/release-policy-hardening/T10-focused-verification.md`
- `sprints/release-policy-hardening/T11-full-release-gate.md`
- `sprints/release-policy-hardening/T12-ci-release-landing.md`

## Findings

- [P1] Early tracker status was too thin for implementation and did not include
  per-track coverage ledgers.
- [P1] Policy integration E2E needed a first-class owner rather than living as
  a vague active-review note.
- [P1] Several verification commands used invalid multi-filter `cargo test`
  shapes and needed to be split.
- [P2] Active reviewer status drifted across `MASTER.md`, `tracker.md`, and
  this file during intake.
- [P1] The shift to `1.1.xxx` and the new T12 release landing gate must be
  synchronized across `MASTER.md`, `tracker.md`, `plan.md`, `swarm.md`, and all
  final finding docs.
- [P1] Frontend runtime/image truth needed an owner after the final swarm wave;
  it is now tracked through T2.8 and T8.6.

## Completed Intake

- [x] UI settings review: transferred to T2.
- [x] Release workflow review: transferred to T0/T5.
- [x] Image builder review: transferred to T1/T4.
- [x] Docs review: transferred to T4.
- [x] capsem-core policy hook review: transferred to T3.
- [x] Service/process review: transferred to T5.
- [x] Logger/session DB review: transferred to T6.
- [x] CLI/update/install review: transferred to T0/T5.
- [x] App/updater shell review: transferred to T0.
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
  `swarm-findings/manual-ui-cli-gates.md`; transferred to T10/T11/tracker.
- [x] CI release landing 1.1 review: captured in
  `swarm-findings/ci-release-landing-1-1.md`; transferred to T9/T11/T12.
- [x] Swarm transfer closeout review: captured in
  `swarm-findings/swarm-transfer-closeout-2026-05-10.md`; transferred to
  T2/T7/T8/T10/T12.

## Active Agents

- [x] None. Final investigation wave is captured in `swarm.md` and
  `swarm-findings/`.

## Task List

### T7.1 Intake Discipline

- [x] When a reviewer returns, classify each finding into T0-T12.
- [ ] Add file-scoped tasks and verification commands to the owning
  sub-sprint.
- [ ] If a finding is deferred, record the reason and owner here.
- [x] Close completed agents after their findings are captured.

### T7.2 Status Synchronization

- [x] Keep `MASTER.md` status synchronized with this file.
- [x] Keep `tracker.md` active swarm section synchronized with actual agents.
- [ ] Keep `plan.md` track order synchronized with T0-T12.
- [ ] Search for stale status text across sprint files after intake changes.

### T7.3 Swarm Depth Gate

- [ ] Each release-blocking track must have at least one focused reviewer or
  explicit reason why code ownership is already covered.
- [ ] Spawn another focused reviewer if any track lacks proof detail.
- [ ] Before implementation starts, run one final sprint-doc QA pass.

### T7.4 Final Closeout

- [ ] Prove no stale `T0-T11` release-control language remains except in
  historical finding docs.
- [ ] Prove no release-facing `1.0.1778378133` target remains except in
  explicit stale-reference checks.
- [ ] Prove each final targeted swarm doc is linked from `swarm.md` and
  `MASTER.md`.
- [ ] Prove every P0/P1 finding in the final targeted docs has an owning task
  in T2/T7/T8/T9/T10/T11/T12.
- [ ] Prove no runnable verification command points at a not-yet-created test
  without an owning creation task.

## Proof Matrix

| Category | Required proof |
|---|---|
| Intake | no completed reviewer result remains only in chat history. |
| Status | active/completed agent status matches actual thread state. |
| Coverage | each T0-T12 file has task list, proof matrix, verification, and exit criteria. |

## Verification

- [ ] `rg -n "In review|Active:" sprints/release-policy-hardening --glob '!T7-active-review-followups.md'`
- [ ] `rg -n "T0-T5|asset_manager policy_hook|policy_hook cleanup" sprints/release-policy-hardening`
- [ ] `rg -n "T0-T11|1\\.0\\.1778378133|pending detailed|proposed split expansion" sprints/release-policy-hardening --glob '!swarm-findings/**'`
- [ ] `git diff --check -- sprints/release-policy-hardening`
- [ ] `rg -n "^## Objective|^## Owned Files|^## Findings|^## Task List|^## Proof Matrix|^## Verification|^## Exit Criteria" sprints/release-policy-hardening/T*.md`

## Exit Criteria

- [x] No completed reviewer result remains only in chat history.
- [ ] Every finding is either actionable in T0-T12 or recorded as deliberately
  deferred.
- [x] Tracker active review slots match actual active subagents.
- [ ] All sub-sprint docs are execution-ready before implementation begins.
