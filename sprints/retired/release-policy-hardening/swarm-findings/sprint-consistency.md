# Sprint Consistency Findings

Status: completed; transferred to T7 FD03 and owner rows in
MASTER/plan/tracker/T7/T10/T11. Downstream implementation remains open.

Agent: Meitner (`019e1263-5600-72d0-9cdc-f19479b74540`)

## Scope

- T0-T11 task ID consistency.
- Status drift across `MASTER.md`, `tracker.md`, and `T7`.
- Missing proof matrices or invalid verification commands.
- Missing sub-sprints or over-broad tracks.

## Findings

- [ ] [P1] `MASTER.md` swarm state is stale. It says T7 dependency is
  `None active`, while `tracker.md` and `T7-active-review-followups.md` list
  active agents.
  - Paths: `sprints/release-policy-hardening/MASTER.md`,
    `sprints/release-policy-hardening/tracker.md`,
    `sprints/release-policy-hardening/T7-active-review-followups.md`.
  - Required change later: add/update `MASTER.md` active swarm state or change
    T7 dependency/notes to match T7.2.
  - Sprint IDs: T7.2.

- [ ] [P1] `MASTER.md` completed swarm intake is incomplete. It stops at the
  second QA swarm, while tracker/T7 include T9, T10/T11, package verification,
  and tracker/doc consistency reviews.
  - Paths: `sprints/release-policy-hardening/MASTER.md`,
    `sprints/release-policy-hardening/tracker.md`,
    `sprints/release-policy-hardening/T7-active-review-followups.md`.
  - Required change later: add those completed-intake bullets under
    `MASTER.md` `## Swarm Inputs Captured`.
  - Sprint IDs: T7.1, T7.2.

- [ ] [P1] `MASTER.md` dependency labels drift from `plan.md`.
  - Paths: `sprints/release-policy-hardening/MASTER.md`,
    `sprints/release-policy-hardening/plan.md`.
  - Required change later: T0 dependency should be `T0.1, T1.1, T5.1`; T3
    should be `None; conditional T8 proof if hook dispatch ships`.
  - Sprint IDs: T7.2.

- [ ] [P2] `MASTER.md` `Proof / Test Count` values do not agree with owning
  verification sections.
  - Paths: `sprints/release-policy-hardening/MASTER.md`,
    `sprints/release-policy-hardening/T9-release-metadata-changelog.md`,
    `sprints/release-policy-hardening/T10-focused-verification.md`.
  - Required change later: replace counts with proof summaries, or regenerate
    counts from each `## Verification`.
  - Sprint IDs: T7.2, T10.7.

- [ ] [P1] `T10-focused-verification.md` has stronger task/proof requirements
  than its command list.
  - Paths: `sprints/release-policy-hardening/T10-focused-verification.md`.
  - Required change later: add exact commands for package payload checks, asset
    cleanup, UI visual proof, policy benchmark smoke, T6 timeline/triage
    checks, T8 E2E, and T9 metadata/version checks.
  - Sprint IDs: T10.1, T10.2, T10.3, T10.4, T10.5, T10.6.

- [ ] [P1] `tracker.md ## Verification Commands` is not a complete rollup of
  track proofs, but it reads like one.
  - Paths: `sprints/release-policy-hardening/tracker.md`,
    `sprints/release-policy-hardening/T6-telemetry-session-tooling.md`,
    `sprints/release-policy-hardening/T8-policy-integration-e2e.md`,
    `sprints/release-policy-hardening/T9-release-metadata-changelog.md`,
    `sprints/release-policy-hardening/T10-focused-verification.md`,
    `sprints/release-policy-hardening/T11-full-release-gate.md`.
  - Required change later: either make it exhaustive or label it as summary and
    point to T6/T8/T9/T10/T11 for required commands.
  - Sprint IDs: T7.2, T10.7, T11.4.

- [ ] [P1] `T7` verification does not prove active/completed swarm sync.
  - Paths: `sprints/release-policy-hardening/T7-active-review-followups.md`.
  - Required change later: add checks comparing active agent names across
    `MASTER.md`, `tracker.md`, and `T7`, and search for stale `None active`
    plus outdated `T0-T8` scope where no longer historical.
  - Sprint IDs: T7.2.

## Positive Check

- Current task IDs/headings in `tracker.md` match the T0-T11 file headings.
  The remaining issues are swarm-state sync and verification-proof
  completeness.
