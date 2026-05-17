# Profile V2 Migration Rescue MASTER

Last updated: 2026-05-17

## Mission
Preserve all critical Profile V2 design and sprint context (including early S00-S06 work) while safely migrating from a drifted dirty state to a trustworthy baseline for continued development.

## Situation Snapshot
- Clean migration line: branch `profile-v2`
- Clean baseline: `origin/main` at `dc137f99` (`release: v1.1.1778860037`)
- Current worktree: `/Users/elie/.codex/worktrees/824d/capsem`
- Source rescue line: detached `HEAD` at `origin/claude/adoring-joliot-98a4cb`
- Source commit: `b3862ae7`
- Source worktree: `/Users/elie/.codex/worktrees/3d94/capsem`
- Concurrency: none (explicit user requirement)

## Context Anchors
Primary sprint corpus to preserve:

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

Adjacent triage context:

- `sprints/profile-v2-test-fix/plan.md`
- `sprints/profile-v2-test-fix/tracker.md`

Rescue inventory:

- `sprints/profile-v2-migration-rescue/rescue-manifest.md`

## Phases
1. **Freeze + Inventory**
   capture exact dirty state and classify changes.
2. **Context Preservation**
   ensure all design/sprint intent is linked before mutation.
3. **Reconciliation**
   keep/drop/review pass with explicit rationale per file.
4. **Verification Recovery**
   rebuild confidence through targeted then broader gates.

## Branch Strategy
- Created dedicated integration branch `profile-v2` from `origin/main`.
- Use `profile-v2` as the clean baseline for resumed Profile V2 delivery.
- Port only classified `keep` changes from the current detached-head line into `profile-v2`.
- Keep the current detached-head commit line as rescue/reference context while migration is in progress.
- Do not wholesale cherry-pick the detached-head line; it diverges from `origin/main` and includes unrelated release/debug/install churn.

## Execution Rules
- Single operator only; no parallel sprinting.
- Do not drop design docs/sprint artifacts without explicit rationale.
- Do not treat generated artifacts as product changes.
- Any test skip or environment gate must be explicitly justified and review-tagged.

## Status
- Active
- Clean branch created
- Profile V2 and rescue sprint documents copied onto `profile-v2`
- Dirty overlay inventory classified in `rescue-manifest.md`
- Core settings profiles, policy confirmation, `/settings*`, debug-report provenance, service runtime VM-effective attachment, capsem-process effective-policy consumption, framed MCP `ask` confirmation, HTTP `ask` confirmation, model `ask` confirmation, model request rewrite, Profile V2 corp-config install, gateway non-VM parity, and VM/MITM Profile V2 policy parity slices replayed on `profile-v2`
- Committed delta classification remains release-held and must be replayed by slice

## Release Holds
- Do not claim migration complete until keep/drop/review manifest exists.
- Do not claim verification restored until remaining broad gates are re-run.
- Do not resume feature delivery on this line until reconciliation pass is complete.
- Ambiguous E2E skip/test loosenings and broad VM gates remain held after the focused VM/MITM parity slice.
