# Swarm Transfer Closeout Findings
Status: completed
Agent: Codex

## Scope

Focused static gap review for swarm intake completeness after the existing
swarm. Reviewed sprint control docs, T0-T11 docs, and existing
`swarm-findings/*.md` for P0/P1 transfer coverage, T12/`1.1.xxx` consistency,
stale `1.0.1778378133` references, invalid or not-yet-backed verification
commands, and proposed work that is not linked to an owning target.

## Findings

- [P0] T12 is partially introduced but not owned consistently.
  - Impact: `MASTER.md` makes T12 the CI/tag/publish release landing gate, and
    T11 now hands off to T12, but `tracker.md`, `plan.md`, and T7 still frame
    execution around T0-T11. CI/live-release work can fall between the local
    release gate and publish gate.
  - Exact paths: `sprints/release-policy-hardening/MASTER.md`,
    `sprints/release-policy-hardening/tracker.md`,
    `sprints/release-policy-hardening/plan.md`,
    `sprints/release-policy-hardening/T7-active-review-followups.md`,
    `sprints/release-policy-hardening/T11-full-release-gate.md`; no
    `sprints/release-policy-hardening/T12-*.md` exists.
  - Owning target: proposed new T12 CI green release landing, plus T7.4 for doc
    synchronization.
  - Required proof: add a T12 owner doc or explicitly fold T12 into T11; update
    tracker execution board/task index, plan track order, and T7 language; prove
    with `rg -n "T0-T11|T12" sprints/release-policy-hardening` and
    `rg --files sprints/release-policy-hardening | rg "T12"`.

- [P0] Swarm transfer is still marked complete in summary areas while detailed
  intake remains pending.
  - Impact: implementation could start with P0/P1 findings still only captured
    as finding-doc bullets, not execution-ready owner tasks with file scopes and
    proof commands.
  - Exact paths: `sprints/release-policy-hardening/swarm.md`,
    `sprints/release-policy-hardening/tracker.md`,
    `sprints/release-policy-hardening/T7-active-review-followups.md`,
    `sprints/release-policy-hardening/swarm-findings/verification-architecture.md`.
  - Owning target: T7.4 swarm closeout, with follow-on edits in affected
    T0-T12 owner docs.
  - Required proof: no remaining `pending detailed`, `pending T`, `proposed
    split expansion`, unchecked dedupe/transfer checklist, or finding-doc P0/P1
    lacking an owner task; prove with targeted `rg` searches over the sprint
    directory.

- [P1] Release version intent still conflicts across active planning docs.
  - Impact: the sprint can still point workers at `1.0.1778378133` while
    `MASTER.md` and T9 say the target release line is `1.1.xxx`, risking wrong
    tag/release metadata or stale release copy.
  - Exact paths: `sprints/release-policy-hardening/tracker.md`,
    `sprints/release-policy-hardening/plan.md`,
    `sprints/release-policy-hardening/T8-policy-integration-e2e.md`,
    `sprints/release-policy-hardening/T9-release-metadata-changelog.md`,
    `sprints/release-policy-hardening/MASTER.md`.
  - Owning target: T9.1 version synchronization, T7.4 doc closeout.
  - Required proof: release-facing planning text consistently says `1.1.xxx`
    until T9 chooses the exact suffix; any remaining `1.0.1778378133` hits are
    explicitly historical/removal checks. Prove with
    `rg -n "1\\.0\\.1778378133|1\\.1\\.xxx" sprints/release-policy-hardening`.

- [P1] Frontend runtime/image truth work is proposed but not linked to an
  executable owner.
  - Impact: asset readiness, image/fork API truth, create defaults, and
    service/gateway status truth can be lost because the UI finding proposes
    new T2/T8/T12 work but existing T2/T8 task lists do not name that slice.
  - Exact paths:
    `sprints/release-policy-hardening/swarm-findings/ui-policy-settings.md`,
    `sprints/release-policy-hardening/swarm-findings/verification-architecture.md`,
    `sprints/release-policy-hardening/T2-frontend-policy-settings.md`,
    `sprints/release-policy-hardening/T8-policy-integration-e2e.md`,
    `sprints/release-policy-hardening/tracker.md`.
  - Owning target: proposed T12 frontend runtime and image truth, or explicit
    T2.8/T8.6 tasks.
  - Required proof: add the owner task(s) with exact UI/API paths and tests for
    asset unknown state, `/images`/image selector contract, and create defaults;
    prove no unresolved `new T2/T8 asset-runtime truth task` or
    `new T2/T8 image/fork UI contract task` references remain.

- [P1] Some verification commands name tests/files that do not exist yet.
  - Impact: T10/T11 can look executable while still depending on future test
    files; a worker may run the rollup and fail for harness drift rather than
    product defects.
  - Exact paths: `sprints/release-policy-hardening/T1-image-manifest-pipeline.md`,
    `sprints/release-policy-hardening/T2-frontend-policy-settings.md`,
    `sprints/release-policy-hardening/T6-telemetry-session-tooling.md`,
    `sprints/release-policy-hardening/T8-policy-integration-e2e.md`,
    `sprints/release-policy-hardening/T10-focused-verification.md`,
    `sprints/release-policy-hardening/swarm-findings/verification-architecture.md`.
  - Owning target: T10.7 evidence capture/command normalization, with the
    creating tests owned by T1/T2/T6/T8.
  - Required proof: either create/rename the referenced tests before they enter
    final verification, or mark them as to-be-created task items rather than
    runnable commands; prove with `test -e` checks or `rg --files` for each
    referenced test path.

## Tests Run

Static review only. Cheap commands run:

- `rg --files sprints/release-policy-hardening`
- `rg -n "T12|1\\.1\\.|1\\.0\\.1778378133|T0-T11|pending detailed|proposed" sprints/release-policy-hardening`
- `rg --files sprints/release-policy-hardening | rg "T12|swarm-transfer-closeout"`
- `just --list | rg '^    (run|exec|doctor|ui|test-install|test)\\b'`
