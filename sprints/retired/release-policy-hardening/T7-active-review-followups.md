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
- [P1] The shift to `1.1.1778445002` and the new T12 release landing gate must be
  synchronized across `MASTER.md`, `tracker.md`, `plan.md`, `swarm.md`, and all
  final finding docs.
- [P1] Frontend runtime/image truth needed an owner after the final swarm wave;
  it is now tracked through T2.8 and T8.6.
- [P0] T7 is the pre-sprint transfer gate for this release hardening work. No
  implementation track may start until every completed finding doc below has a
  durable owner row in the relevant T0-T12 sub-sprint tracker.
- [P2] The final T7 mapping audit found no orphaned P0/P1 findings, but caught
  stale timing/status wording and one conditional missing test path. Those are
  now captured in FD14 and normalized here before T8 starts.

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
  `swarm-findings/sprint-consistency.md`; FD03 owner rows added in
  T7/T10/T11.
- [x] Final core policy/assets review: captured in
  `swarm-findings/core-policy-assets.md`; FD04 owner rows added in
  T1/T3/T6/T8/T10.
- [x] Final service/process review: captured in
  `swarm-findings/service-process.md`; FD05 owner rows added in T3/T5/T8/T10.
- [x] Final CLI/install/updater review: captured in
  `swarm-findings/cli-updater-install.md`; FD06 owner rows added in
  T0/T5/T9/T10/T11.
- [x] Final MCP policy boundary review: captured in
  `swarm-findings/mcp-policy-boundary.md`; FD07 owner rows added in
  T3/T5/T6/T8/T10.
- [x] Final telemetry/session review: captured in
  `swarm-findings/telemetry-session.md`; FD08 owner rows added in
  T3/T6/T8/T10.
- [x] Final guest/image-builder review: captured in
  `swarm-findings/guest-image-builder.md`; FD09 owner rows added in T1/T5/T10.
- [x] Final CI packaging review: captured in
  `swarm-findings/ci-packaging.md`; FD10 owner rows added in
  T0/T1/T5/T10/T11/T12.
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
- [x] T7 transfer mapping audit: captured in
  `swarm-findings/swarm-transfer-closeout-2026-05-10.md`; confirmed no
  orphaned P0/P1 findings and transferred stale-status/command-validity fixes
  to T7/T8/tracker/MASTER.

## Finding Doc Transfer Subtasks

Each item below is a pre-sprint subtask. The first checkbox means the finding
doc was read in full during T7 intake. The second means every point from that
doc has an owner row in at least one T0-T12 `## Swarm Transfer Tracker`
section. Implementation work resolves the downstream T-track checkboxes; T7
only protects against losing or relying on chat-only findings.

- [x] FD01 `swarm-findings/ui-policy-settings.md`
  - [x] Read complete.
  - [x] Owner rows added in T2, T8, and T10.
  - [ ] Downstream blockers resolved: hook UI scope, callback/runtime support
    matrix, settings reload truth, asset health unknown state, image/fork UI
    contract, create defaults, mock-settings drift, and Gate A proof.
- [x] FD02 `swarm-findings/docs-release-metadata.md`
  - [x] Read complete.
  - [x] Owner rows added in T0, T4, T6, T9, T10, T11, and T12.
  - [ ] Downstream blockers resolved: hook overclaim cleanup, stale
    artifact/updater docs, Tauri updater strategy, telemetry docs, public site
    stale claims, curated release text, artifact truth in release notes, and
    manifest-signing preflight.
- [x] FD03 `swarm-findings/sprint-consistency.md`
  - [x] Read complete.
  - [x] Owner rows added in T7, T10, and T11.
  - [ ] Downstream blockers resolved: status/dependency drift, proof-summary
    drift, incomplete T10 commands, non-exhaustive tracker rollup, and
    active/completed swarm sync checks.
- [x] FD04 `swarm-findings/core-policy-assets.md`
  - [x] Read complete.
  - [x] Owner rows added in T1, T3, T6, T8, and T10.
  - [ ] Downstream blockers resolved: MCP notification bypass, hook
    production-scope decision, response body cap, loopback lookalikes,
    fail-closed semantics, Spec0 semantic validation, hook fallback audit
    rows, asset version ordering, cleanup, MCP telemetry naming, and benchmark
    guardrails.
- [x] FD05 `swarm-findings/service-process.md`
  - [x] Read complete.
  - [x] Owner rows added in T3, T5, T8, and T10.
  - [ ] Downstream blockers resolved: MCP helper install/discovery, settings
    apply/reload semantics, builtin MCP startup-only policy, `McpRefreshTools`
    builtin wiring, helper env isolation, deterministic cleanup, hook dispatch
    integration decision, and `/policy-hook/spec` route/auth coverage.
- [x] FD06 `swarm-findings/cli-updater-install.md`
  - [x] Read complete.
  - [x] Owner rows added in T0, T5, T9, T10, and T11.
  - [ ] Downstream blockers resolved: clean install signed-manifest contract,
    Linux MCP helpers, postinstall failure semantics, verified manifest
    consumers, Tauri updater honesty, package-installed freshness proof, update
    UI truth, version/update copy, and tag hold.
- [x] FD07 `swarm-findings/mcp-policy-boundary.md`
  - [x] Read complete.
  - [x] Owner rows added in T3, T5, T6, T8, and T10.
  - [ ] Downstream blockers resolved: no-id `tools/call` bypass, Linux helper
    packaging, stdio env leakage, redirect-time builtin policy, refresh
    builtin wiring, builtin denial telemetry, MCP action naming, trace
    propagation, and gateway route/auth proof.
- [x] FD08 `swarm-findings/telemetry-session.md`
  - [x] Read complete.
  - [x] Owner rows added in T3, T6, T8, and T10.
  - [x] Downstream blockers resolved: timeline/triage Policy V2 visibility,
    old-DB compatibility, current schema checks, MCP/tool correlation SQL,
    hook failure audit rows, reader/frontend trace and policy visibility.
- [x] FD09 `swarm-findings/guest-image-builder.md`
  - [x] Read complete.
  - [x] Owner rows added in T1, T5, and T10.
  - [ ] Downstream blockers resolved: release-built initrd sysutil coverage,
    hard rootfs validation, `_pack-initrd` two-arch manifest preservation,
    same-day version unification, per-arch cleanup, stale binary test
    contracts, and guest artifact permissions.
- [x] FD10 `swarm-findings/ci-packaging.md`
  - [x] Read complete.
  - [x] Owner rows added in T0, T1, T5, T10, T11, and T12.
  - [ ] Downstream blockers resolved: `.pkg` manifest contract, `.deb`
    manifest contract, Linux MCP helpers, Linux/rootfs release-blocking CI,
    binary metadata preservation, updater artifact strategy, manifest-signing
    preflight, package-payload post-release proof, and provenance.
- [x] FD11 `swarm-findings/verification-architecture.md`
  - [x] Read complete.
  - [x] Owner rows added in T2, T7, T8, T10, T11, and T12.
  - [ ] Downstream blockers resolved: swarm completeness, invalid commands,
    T8 scope fork, frontend runtime/image truth owner, T10.8 evidence ledger,
    T11/T12 release verification split, and missing-test ownership.
- [x] FD12 `swarm-findings/manual-ui-cli-gates.md`
  - [x] Read complete.
  - [x] Owner rows added in T10, T11, and tracker evidence ledger.
  - [ ] Downstream blockers resolved: final `just install` local package gate,
    valid `just exec` command replacement, Gate A-D blocking checklist rows,
    evidence schema, dev desktop proof, and installed app full-launch proof.
- [x] FD13 `swarm-findings/ci-release-landing-1-1.md`
  - [x] Read complete.
  - [x] Owner rows added in T1, T5, T9, T11, and T12.
  - [ ] Downstream blockers resolved: `1.1.1778445002` stamping, T12 ownership, Linux
    release-blocking CI, stale companion-binary package contract, rootfs
    validation parity, updater incompatibility, and local release-check script
    coverage.
- [x] FD14 `swarm-findings/swarm-transfer-closeout-2026-05-10.md`
  - [x] Read complete.
  - [x] Owner rows added in T2, T7, T8, T9, T10, and T12.
  - [ ] Downstream blockers resolved: T12 consistency, no summary-complete
    while detail-pending drift, `1.1.1778445002` planning consistency, frontend
    runtime/image truth owner, and nonexistent-test command normalization.

## Active Agents

- [x] None. Final investigation wave is captured in `swarm.md` and
  `swarm-findings/`.

## Task List

### T7.1 Intake Discipline

- [x] When a reviewer returns, classify each finding into T0-T12.
- [x] Add file-scoped tasks and verification commands to the owning
  sub-sprint.
- [x] If a finding is deferred, record the reason and owner here. No finding
  was deliberately deferred during pre-sprint transfer; implementation blockers
  remain open in their owner tracks.
- [x] Close completed agents after their findings are captured.

### T7.2 Status Synchronization

- [x] Keep `MASTER.md` status synchronized with this file.
- [x] Keep `tracker.md` active swarm section synchronized with actual agents.
- [x] Keep `plan.md` track order synchronized with T0-T12.
- [x] Search for stale status text across sprint files after intake changes.

### T7.3 Swarm Depth Gate

- [x] Each release-blocking track must have at least one focused reviewer or
  explicit reason why code ownership is already covered.
- [x] Spawn another focused reviewer if any track lacks proof detail. Final
  read-only mapping reviewer Galileo found no P0/P1 transfer gaps.
- [x] Before moving from intake to T8, run one final sprint-doc QA pass.

### T7.4 Final Closeout

- [x] Prove no stale `T0-T11` release-control language remains except in
  historical finding docs.
- [x] Prove no release-facing `1.0.1778378133` target remains except in
  explicit stale-reference checks.
- [x] Prove each final targeted swarm doc is linked from `swarm.md` and
  `MASTER.md`.
- [x] Prove every P0/P1 finding in the final targeted docs has an owning task
  in T2/T7/T8/T9/T10/T11/T12.
- [x] Prove no runnable verification command points at a not-yet-created test
  without an owning creation task.
- [x] Prove every `swarm-findings/*.md` status has been updated from
  pre-transfer language after downstream owner rows and proof tasks were
  created.

## Proof Matrix

| Category | Required proof |
|---|---|
| Intake | no completed reviewer result remains only in chat history. |
| Status | active/completed agent status matches actual thread state. |
| Coverage | each T0-T12 file has task list, proof matrix, verification, and exit criteria. |

## Verification

- [x] `rg -n "\\| In progress \\|" sprints/release-policy-hardening/swarm.md`
  (no matches).
- [x] `rg -n "Awaiting agent output" sprints/release-policy-hardening/swarm-findings`
  (no matches).
- [x] `rg -n "before implementation starts|before implementation begins|pre-implementation" sprints/release-policy-hardening --glob '!sprints/release-policy-hardening/swarm-findings/**' --glob '!sprints/release-policy-hardening/T7-active-review-followups.md' --glob '!sprints/release-policy-hardening/swarm.md'`
  (no matches).
- [x] `rg -n "T0-T11|pending detailed|proposed split expansion" sprints/release-policy-hardening --glob '!sprints/release-policy-hardening/swarm-findings/**' --glob '!sprints/release-policy-hardening/T7-active-review-followups.md'`
  (no matches).
- [x] `rg -n "1\\.0\\.1778378133" README.md CHANGELOG.md LATEST_RELEASE.md docs/src/content/docs site/src crates frontend scripts src config .github`
  (only historical `CHANGELOG.md` and `docs/src/content/docs/releases/1-0.md`
  matches remain after T9 selected `1.1.1778445002`).
- [x] `rg -n "pending transfer|pending expansion|pending detailed" sprints/release-policy-hardening --glob '!sprints/release-policy-hardening/swarm-findings/**' --glob '!sprints/release-policy-hardening/T7-active-review-followups.md'`
  (no matches).
- [x] `rg -n "new T2/T8|proposed T12|does not exist yet" sprints/release-policy-hardening --glob '!sprints/release-policy-hardening/swarm-findings/**' --glob '!sprints/release-policy-hardening/T7-active-review-followups.md'`
  (no matches).
- [x] `for p in ...; do test -e "$p" || printf 'MISSING %s\n' "$p"; done`
  over runnable script/test paths referenced from T0-T12, `MASTER.md`, and
  `tracker.md` (only `tests/capsem-e2e/test_policy_hook_runtime.py` is
  missing, and T8.2 explicitly owns creating it before the conditional
  hook-ships command can enter a final gate).
- [x] `git diff --check -- sprints/release-policy-hardening`.
- [x] `rg -n "^## Objective|^## Owned Files|^## Findings|^## Task List|^## Proof Matrix|^## Verification|^## Exit Criteria" sprints/release-policy-hardening/T*.md`
  (all T0-T12 docs expose the expected sections).

## Exit Criteria

- [x] No completed reviewer result remains only in chat history.
- [x] Every finding is either actionable in T0-T12 or recorded as deliberately
  deferred.
- [x] Tracker active review slots match actual active subagents.
- [x] All remaining sub-sprint docs are execution-ready before T8 starts; the
  only missing referenced test file is conditionally owned by T8.2.
- [ ] All FD01-FD14 downstream blocker checkboxes above are either resolved by
  T0-T12 implementation or deliberately deferred with an owner.
