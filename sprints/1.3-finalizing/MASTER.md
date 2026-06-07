# 1.3 Finalizing Master

This is the coordination page for closing 1.3 after the profile/API/security
contract reset.

## Workstreams

| Stream | Status | Notes |
| --- | --- | --- |
| T0 Schema and ownership | Not Started | Profile/settings/corp schemas, immutable VM profile id, defaults/plugin/credential contract. |
| T1 Service/gateway API | In Progress | Profile plugin, MCP server/tool, enforcement authoring, full `/corp/info|edit|validate|reload`, `/settings/info|edit`, profile reload, VM ledger routes, VM core/lifecycle routes, and VM utility routes now live under `/vms...`; retired plugin global/VM, global MCP, global enforcement authoring, `/corp-config`, `GET|POST /settings`, `/settings/lint`, `/settings/validate-key`, `/settings/presets`, `/reload-config`, old ledger routes, and old top-level VM routes fail closed. Other authoring routes still need profile burn-down. |
| T2 Security rail burn-down | In Progress | Network web decision settings and MCP policy objects burned; remaining work is route/authoring/profile completion plus full invariant sweep. |
| T3 Profile/settings/corp UI/API split | Not Started | Settings UI-only, profile behavior profile-backed, one editor writes one contract. |
| T4 MCP/plugins/credentials/skills UI | In Progress | Plugin UI/API use profile routes; MCP tools now load under profile/server routes. MCP resources/prompts, credentials, and skills remain. |
| T5 VM lifecycle/assets/install | Blocked | Snapshot loss must be repaired: profile catalog/assets/pins, `capsem-admin`, profile-derived EROFS/LZ4HC asset builds, TUI/terminal shell, Linux/KVM proof, and security corpus/benchmark gates all need restore/port decisions before 1.3 can close. See `profile-platform-lost-work-audit.md`. |
| T6 Docs/changelog/skills | Not Started | Full docs pass, changelog, skills, benchmark docs. |
| T6.5 Invariant review | Not Started | Full pre-verification review of every master contract invariant. |
| T7 Release verification | Not Started | Focused tests, full smoke, full test cycle, full install cycle, UI sanity, benchmark check. |

## Ground Rules

- Current main/worktree truth stays authoritative.
- Do not resurrect old policy-v2 paths.
- Burn old authoring APIs and old decision engines. No fallbacks, no
  compatibility aliases, no "if old shape then..." runtime escape hatches.
- Remove dead code instead of quarantining it.
- Every security/config/API slice needs adversarial tests proving old shapes and
  bypass attempts fail closed.
- Do not add `NetworkRouting`.
- Linux-team scoped KVM/filesystem/EROFS/benchmark work is authoritative for
  1.3. Restore or port those commits in their scoped files unless they directly
  violate the current security/profile contract; do not silently drop them as
  merge noise.
- Network engine owns mechanics: parsing, capture, DNS/proxy mechanics, ports,
  caching, decompression, routing mechanics, provider metadata.
- Network engine does not own security decisions.
- MCP owns server/tool/resource/prompt config and discovery mechanics.
- MCP does not own security decisions.
- Allow/ask/block/rewrite/preprocess/postprocess decisions remain CEL/security
  rule decisions over typed security events.
- Default rules are visible real rules in the same `SecurityRuleSet`; no second
  default engine.
- A VM executes one immutable profile id.
- Profile owns VM behavior: assets, VM config, rules, detections, MCP, skills,
  credentials/plugins, availability, name, description, icon/SVG.
- `settings.toml` owns UI/application preferences only.
- Corp owns constraints, locks, reporting, and integrations over profiles.
- One UI editor surface writes one backing contract.
- UI reflects backend contracts and does not invent config copy.
- Service-global endpoints may only report runtime/service/ledger state.

## Contract Drafts

- [api-contract.md](api-contract.md) is the current endpoint contract draft.
- [plan.md](plan.md) contains the required end posture and security/UI contracts.
- [model-breakage-audit.md](model-breakage-audit.md) captures the initial breakage audit.
- [profile-platform-lost-work-audit.md](profile-platform-lost-work-audit.md)
  captures the profile catalog/assets/pins/launchability work that was lost or
  flattened during cleanup.
- [snapshot-restore/MASTER.md](snapshot-restore/MASTER.md) tracks the focused
  restore sub-sprint and commit inspection ledger.
- [tracker.md](tracker.md) is the live execution checklist.

## Release Gate

Release is blocked until:

- T0-T6 implementation/docs slices are complete and committed.
- T6.5 invariant review is complete and any findings are fixed/committed.
- T7 verification passes.
- Changelog matches implemented behavior.
- Full smoke, full tests, full install cycle, and UI sanity pass are recorded.
- Linux-only validation items are either passed by the Linux team or explicitly
  documented as Linux handoff blockers.
