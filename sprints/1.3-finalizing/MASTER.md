# 1.3 Finalizing Master

This is the coordination page for closing 1.3 after the security-rule/defaults
discussion.

## Workstreams

| Stream | Status | Notes |
| --- | --- | --- |
| Security rule defaults | Paused | Need final decision on `profiles.defaults` and override semantics. |
| Plugin contract | Paused | Need exact required built-in plugin list and reachability invariant. |
| Profile contract | Paused | Need canonical profile schema: VM executes profile; settings are UI-only; corp constrains/reporting. |
| Enforcement/detection API | Paused | Must become profile-addressed; global `/enforcements/list` is not the final model. |
| Policy UI | Paused | Must reflect backend rule names/reasons; no invented copy. |
| Old policy burn pass | Pending | Re-check old domain/MCP decision remnants after defaults settle. |
| Release verification | Pending | Tests, smoke, docs, changelog, Linux handoff. |

## Ground Rules

- Current main/worktree truth stays authoritative.
- Do not resurrect old policy-v2 paths.
- Do not add `NetworkRouting`.
- Network cache, parsing, DNS redirects, port mechanics, and body capture remain network-engine mechanics.
- Allow/ask/block decisions remain rule/CEL decisions.
- UI reflects backend contracts and does not invent rule/plugin descriptions.
- A VM executes a profile.
- Profile owns VM behavior: assets, VM/runtime config, rules, detections, MCP, skills, provider/model config.
- Settings are UI/application preferences only.
- Corp owns constraints, locks, reporting, and integrations over profiles.
- Only service-global endpoints may be global.

## Contract Draft

- [api-contract.md](api-contract.md) is the current endpoint contract draft.
