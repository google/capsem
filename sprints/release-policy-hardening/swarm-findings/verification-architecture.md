# Verification Architecture Findings

Status: completed; transferred to T7 FD11 and owner rows in
T2/T7/T8/T10/T11/T12. Downstream implementation remains open.

Agent: Cicero (`019e126a-d865-7ce3-9ec6-cc9b15637250`)

## Scope

- Missing sub-sprints.
- Invalid commands.
- Proof gaps.
- Over-broad tracks.
- Release-gate holes.
- Implementation-readiness blockers.

## Findings

- [ ] [P0] Implementation is not ready until swarm completeness is closed.
  - Release impact: moving into implementation now would lose active findings
    and leave tracker state stale.
  - Paths: `sprints/release-policy-hardening/swarm.md:173`,
    `sprints/release-policy-hardening/tracker.md:29`,
    `sprints/release-policy-hardening/T7-active-review-followups.md:67`.
  - Detail: at review time, several finding docs were placeholders and
    tracker/T7 still listed old swarm state.
  - Proof: no placeholder agent-output text, no `In progress` rows, and
    synchronized `MASTER.md`/`tracker.md`/T7 before implementation.
  - Sprint IDs: T7.4, T10.7, T11.4.

- [ ] [P0] Several verification commands are not implementation-ready.
  - Release impact: final gate can reference nonexistent commands/tests and
    create false confidence.
  - Paths: `sprints/release-policy-hardening/MASTER.md:39`,
    `sprints/release-policy-hardening/plan.md:71`,
    `sprints/release-policy-hardening/T11-full-release-gate.md:48`,
    `justfile`.
  - Detail: `just run "capsem-doctor"` appears in sprint docs, but the justfile
    has `exec`, `run-service`, `smoke`, and `doctor`, not `run`. Referenced
    tests that do not exist yet include
    `frontend/src/lib/__tests__/policy-rules-section.test.ts`,
    `tests/capsem-e2e/test_policy_hook_runtime.py`, and
    `tests/capsem-session/test_check_session_compat.py`.
  - Proof: normalize to `just exec "capsem-doctor"` or define intended recipe;
    create/rename missing tests before listing them as executable.
  - Sprint IDs: T10.7, T11.2, T11.3.

- [ ] [P1] T8 is a decision fork, not one implementable sprint.
  - Release impact: keeping both hook-shipping branches open will keep UI,
    docs, runtime, and telemetry drifting.
  - Paths: `sprints/release-policy-hardening/T8-policy-integration-e2e.md:44`.
  - Detail: T8.1 must become a hard gate before T2/T3/T4/T6 proceed: either
    hook dispatch ships with endpoint config, reload, runtime dispatch,
    telemetry, and E2E proof, or hook UI/docs are hidden/deferred.
  - Proposed split: `T8.0 shipping-scope decision`, `T8A hook dispatch ships`,
    `T8B hook dispatch deferred`, `T8C running-session apply semantics`,
    `T8D callback/runtime support matrix`.

- [ ] [P1] Frontend runtime/image truth findings need their own sub-sprint.
  - Release impact: asset readiness, image/fork UI, create defaults, and
    service/gateway status truth can get buried under Policy settings.
  - Paths:
    `sprints/release-policy-hardening/swarm-findings/ui-policy-settings.md:51`.
  - Proposed split: add `T12 frontend runtime and image truth`, or explicit
    `T2.8/T8.6` tasks for asset health, image/fork UI contract, create
    defaults, and service/gateway status truth.

- [ ] [P1] T10 is not a complete focused-verification rollup.
  - Release impact: focused verification will miss package payloads, asset
    cleanup, visual proof, benchmark smoke, T6 timeline/triage, T8 E2E, and T9
    metadata.
  - Paths:
    `sprints/release-policy-hardening/T10-focused-verification.md:98`,
    `sprints/release-policy-hardening/swarm-findings/sprint-consistency.md`.
  - Proposed split: add `T10.8 evidence ledger` with command, expected proof,
    pass/fail, artifact/log path, owner, and release-blocking follow-up.

- [ ] [P1] T11 lacks post-publish release verification owner.
  - Release impact: preflight/full-suite/install smoke can pass while live
    GitHub release assets or notarization checks are broken.
  - Paths: `sprints/release-policy-hardening/T11-full-release-gate.md:76`.
  - Proposed split: add `T11.6 post-release verification` covering
    `gh release view/download`, `.pkg` signature, Gatekeeper, stapler
    validation, live `.pkg`/`.deb` payload verification, and clean install.

## Proposed Splits

- [ ] Add `T7.4 swarm closeout`: reconcile `swarm.md`, finding-doc statuses,
  `MASTER.md`, `tracker.md`, and T7 before implementation.
- [ ] Split T0 into package manifest/payload, verified manifest consumers,
  postinstall failure semantics, and updater strategy.
- [ ] Split T5 into helper packaging, environment isolation, cleanup, and
  reload semantics; rootfs validation should be owned primarily by T1 with T5
  as dependency.
- [ ] Split T8 into decision gate plus ship/defer branches and runtime support
  matrix.
- [ ] Add `T12 frontend runtime and image truth`, or explicit `T2.8/T8.6`.
- [ ] Add `T10.8 evidence ledger`.
- [ ] Add `T11.6 post-release verification`.

## Files To Update Later

- `sprints/release-policy-hardening/swarm.md`
- `sprints/release-policy-hardening/swarm-findings/verification-architecture.md`
- all placeholder finding docs
- `sprints/release-policy-hardening/MASTER.md`
- `sprints/release-policy-hardening/tracker.md`
- `sprints/release-policy-hardening/plan.md`
- `sprints/release-policy-hardening/T7-active-review-followups.md`
- `sprints/release-policy-hardening/T0-release-artifacts.md`
- `sprints/release-policy-hardening/T2-frontend-policy-settings.md`
- `sprints/release-policy-hardening/T5-service-process-packaging.md`
- `sprints/release-policy-hardening/T8-policy-integration-e2e.md`
- `sprints/release-policy-hardening/T10-focused-verification.md`
- `sprints/release-policy-hardening/T11-full-release-gate.md`

## Blockers

- [ ] Complete active swarm outputs.
- [ ] Resolve hook shipping scope.
- [ ] Resolve updater strategy.
- [ ] Decide macOS clean-install CI vs manual proof.
- [ ] Normalize invalid commands.
- [ ] Add or rename missing tests.
- [ ] Assign implementation plus verification owners per release-blocking track.

## Tests Not Run

- Static review only; no tests were run.
