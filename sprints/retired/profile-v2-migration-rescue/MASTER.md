# Profile V2 Migration Rescue MASTER

Last updated: 2026-05-18

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
- Core settings profiles, policy confirmation, `/settings*`, debug-report provenance, service runtime VM-effective attachment, capsem-process effective-policy consumption, framed MCP `ask` confirmation, HTTP `ask` confirmation, model `ask` confirmation, model request rewrite, Profile V2 corp-config install, gateway non-VM parity, VM/MITM Profile V2 policy parity, and `just smoke` ordering/runtime rescue slices replayed on `profile-v2`
- S00-S19 merged-code audit added at `sprints/profile-v2-migration-rescue/audit.md`
- capsem-process V1 `user.toml`/`MergedPolicies` runtime bridge removed; focused RED/GREEN guardrails now assert Profile V2-only runtime authority, guest boot assembly, and DNS/full-block `NetworkPolicy` conversion
- Smoke integration now uses a temporary Profile V2 service/profile fixture instead of removed `CAPSEM_USER_CONFIG`/`CAPSEM_CORP_CONFIG` runtime policy plumbing
- S06 hygiene closeout is green: guest boot config is canonical under `vm::guest_config`, the old policy-config guest-config export is guarded against, deterministic default MCP injection no longer reads process-wide V1 config, and Docker install E2E handles symlinked asset roots correctly.
- Generated builder/frontend settings fixtures are quarantined from runtime
  authority: guard tests assert Rust crates do not embed `defaults.json` or
  `settings-schema.json`, and stale `policy_config`/V1 wording has been removed
  from the builder, frontend mock, and MITM comments.
- S07 has started with the typed metrics IPC foundation:
  `capsem_proto::metrics`, `ServiceToProcess::GetMetricsSnapshot`, and
  `ProcessToService::MetricsSnapshot` compile and round-trip over bincode.
- S07 now also includes UDS Profile V2 profile CRUD/resolve plus rules
  list/get/evaluate. Rules create/delete, confirm listing, skills, gateway
  mirror, and route-level Python/VM proof remain release-held.
- `just smoke` passed on 2026-05-17 after the Profile V2 runtime/DNS rescue (`just smoke`, 224s)
- `just test-install` passed on 2026-05-18 after the asset symlink/mount and file-only copy fixes (`57 passed`, `29 skipped`)
- Committed delta classification remains release-held and must be replayed by slice

## Release Holds
- Do not claim migration complete until keep/drop/review manifest exists.
- Do not claim S01 public cleanup complete until setup/install/docs `user.toml` references are replaced or explicitly quarantined in the S19 docs/API work.
- Do not claim S07-S19 complete; `audit.md` currently marks most public API/UI/CLI/OTel/docs surfaces as partial or gap.
- Do not claim final release readiness until S07-S19 public surfaces and docs are implemented and verified.
- Do not resume feature delivery on this line until reconciliation pass is complete.
- Ambiguous E2E skip/test loosenings remain held after the focused VM/MITM parity and full smoke gates.
