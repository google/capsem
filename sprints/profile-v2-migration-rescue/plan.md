# Profile V2 Migration Rescue Sprint Plan

Last updated: 2026-05-17

## Sprint Name
`profile-v2-migration-rescue`

## Situation Reference
This repository is currently in a mixed state after significant Profile V2 work and follow-on test triage:

- Git state: **detached HEAD** at `origin/claude/adoring-joliot-98a4cb`
- Commit pin: `b3862ae7`
- Working tree: dirty with implementation edits, test edits, and generated artifacts
- Risk: high chance of losing product/design context if we do ad hoc cleanup

User direction for this sprint:

- Preserve full Profile V2 design/implementation context
- No concurrent sprinting
- Create a clean branch from main for the rescue
- Build a deterministic migration plan and execution record so we do not lose track

## Authoritative Context Documents
Primary profile/settings sprint corpus:

- `sprints/policy-settings-profiles/MASTER.md`
- `sprints/policy-settings-profiles/plan.md`
- `sprints/policy-settings-profiles/tracker.md`
- `sprints/policy-settings-profiles/S00-meta-sprint-setup.md`
- `sprints/policy-settings-profiles/S01-remove-v1-settings-policy.md`
- `sprints/policy-settings-profiles/S02-service-settings-design.md`
- `sprints/policy-settings-profiles/S03-service-settings-implementation.md`
- `sprints/policy-settings-profiles/S04-profile-design.md`
- `sprints/policy-settings-profiles/S05-profile-implementation.md`
- `sprints/policy-settings-profiles/S06-assembly-vm-effective-settings.md`
- `sprints/policy-settings-profiles/S00-S06-audit-2026-05-14.md`

Related immediate triage sprint:

- `sprints/profile-v2-test-fix/plan.md`
- `sprints/profile-v2-test-fix/tracker.md`

## Objective
Create a safe, auditable migration path that preserves all Profile V2 design and early sprint implementation intent while removing accidental drift/noise and restoring trustworthy TDD verification.

## Scope
In scope:

- Capture and pin current state (branch/HEAD/files changed)
- Create clean `profile-v2` branch from `origin/main`
- Build keep/drop classification for modified files
- Preserve design and sprint artifacts with explicit references
- Define migration sequencing for code and tests
- Define verification gates and release hold criteria

Out of scope (for this sprint doc phase):

- Large-scale refactor or feature additions
- Parallel multi-agent swarming
- Wholesale cherry-pick of the detached source line

## Naming and Ownership
- Sprint directory: `sprints/profile-v2-migration-rescue/`
- Mode: single operator, single thread, no concurrent sprint
- Clean migration line: `profile-v2` at `origin/main` baseline `dc137f99`
- Current source line: detached head `b3862ae7` (`origin/claude/adoring-joliot-98a4cb`)

## Migration Strategy
1. Freeze: snapshot current state and produce a rescue manifest.
2. Classify: separate `keep`, `drop`, `needs-review` changes.
3. Preserve context first: ensure all profile sprint docs and decisions are linked.
4. Reconcile code second: keep only intentional Profile V2 and supporting fixes.
5. Verify incrementally: targeted tests -> suite slices -> full gate.

## Done Criteria
- `MASTER.md`, `plan.md`, `tracker.md` are synchronized.
- Current branch/commit/situation are explicitly documented.
- Profile sprint document references (S00-S06 + audit/meta) are explicitly documented.
- Dirty overlay keep/drop/review workflow is defined with verification gates.
- Committed delta is replayed slice-by-slice with file-level decisions, not applied wholesale.
- No concurrency assumptions remain.

## Proof Matrix
- Unit/contract: migration classification notes for unit-touching files
- Functional: targeted Profile V2 E2E and service/gateway slices
- Adversarial: explicit handling of flaky/environment-coupled tests
- E2E/VM or integration: doctor/network-dependent cases called out with capability gating
- Telemetry/observability: net/dns/policy event coverage retained
- Performance: benchmark changes classified as keep/drop/noise
