# Sprint: 1.3 Finalizing

## Status

Paused for discussion. Do not continue implementation until the design questions
in `plan.md` are resolved.

## Immediate Next Conversation

- [x] Draft profile-first API contract in `api-contract.md`.
- [x] Burn approved endpoint/profile posture into `plan.md` as release requirement.
- [x] Burn security ownership contract into `plan.md`: network/MCP mechanics only, security decisions only on CEL/rules, defaults are real visible rules.
- [x] Burn UI reflection contract into `plan.md` and `skills/dev-capsem/SKILL.md`.
- [ ] Define the canonical profile schema and VM-executes-profile contract.
- [ ] Identify which current settings are profile-owned versus UI-owned.
- [ ] Review and accept/revise the profile-addressed route shape for enforcement, detection, plugins, MCP, assets, and skills.
- [ ] Decide whether `profiles.defaults.*` is the final visible grouping.
- [ ] Decide default rule override semantics.
- [ ] Decide `/profiles/{profile_id}/enforcement/rules` response shape.
- [ ] Decide whether detection remains a parallel `/profiles/{profile_id}/detection/rules` endpoint family for 1.3.
- [ ] Decide how much UI editing belongs in 1.3 versus follow-up.

## Current Partial Work To Reconcile

- [ ] Review uncommitted compiler/default-rule changes.
- [ ] Review uncommitted service/gateway `/enforcements/list` changes and likely reshape/remove in favor of profile-addressed routes.
- [ ] Review uncommitted frontend Policy section changes.
- [ ] Decide whether to keep, reshape, or revert `sprints/security-default-rule-rail/`.
- [ ] Reconcile code against `api-contract.md`.

## Model Breakage Audit

- [x] Audit service routes for profile-less authoring endpoints and ambiguous `info`/`status` use.
- [x] Audit gateway forwarding/routes for profile-less authoring endpoints.
- [x] Audit frontend API helpers and UI pages for settings-owned VM behavior.
- [x] Audit config/profile/settings/corp parsing for ownership violations.
- [x] Audit MCP assumptions for global tool/resource/prompt lists.
- [x] Audit credential/provider assumptions for remaining provider API objects.
- [x] Audit VM lifecycle assumptions for immutable profile id, pause/resume/save/fork/status.
- [ ] Audit docs/skills for old endpoint/config mental model.
- [x] Capture initial findings in `model-breakage-audit.md`.

## Documentation Updates

- [x] Added REST endpoint vocabulary and profile/settings/corp ownership rules to `skills/dev-capsem/SKILL.md`.

## Release Holds

- [ ] No release until default-rule grouping is contract-tested.
- [ ] No release until profile/settings/corp ownership is codified in docs and code.
- [ ] No release until MCP and network decision ownership violations are removed.
- [ ] No release until UI profile/security/plugin/MCP pages reflect backend contract fields without invented config copy.
- [ ] No release until plugin/default profile invariants are tested.
- [ ] No release until frontend Policy UI is either completed or intentionally removed from 1.3.
- [ ] No release until changelog/docs match implemented behavior.

## Coverage Ledger

- Unit/contract: pending.
- Functional: pending.
- Adversarial: pending.
- E2E/VM: pending.
- Telemetry/session DB: pending.
- Frontend: pending.
- Performance: unchanged in this sprint unless benchmarks are rerun.
