# Sprint: 1.3 Finalizing

## Status

Contract approved enough to start cleanup implementation. Keep committing
functional slices steadily. Do not batch unrelated fixes into one giant release
commit.

## Burn Discipline

- [ ] No fallback routes for old authoring APIs.
- [ ] No compatibility aliases for old authoring APIs.
- [ ] No hidden branch that accepts both old and new ownership models.
- [ ] No "if old shape then..." runtime escape hatches.
- [ ] Remove dead code instead of quarantining it.
- [ ] Tests must prove old paths/shapes fail closed.
- [ ] Adversarial tests are required for every security/config/API slice.
- [ ] Changelog/docs must describe the new contract, not migration folklore.

## Contract Baseline

- [x] Draft profile-first API contract in `api-contract.md`.
- [x] Burn endpoint/profile posture into `plan.md`.
- [x] Burn security ownership contract into `plan.md`: network/MCP mechanics
  only, security decisions only on CEL/rules, defaults are real visible rules.
- [x] Burn UI reflection contract into `plan.md` and `skills/dev-capsem/SKILL.md`.
- [x] Burn one-UI-editor-one-contract rule into docs.
- [x] Audit model breaks and capture them in `model-breakage-audit.md`.

## Current Partial Work To Reconcile

- [ ] Review uncommitted compiler/default-rule changes.
- [ ] Review uncommitted service/gateway `/enforcements/list` changes and
  remove in favor of profile-addressed routes.
- [ ] Review uncommitted frontend Policy section changes.
- [ ] Decide whether to keep, reshape, or remove `sprints/security-default-rule-rail/`.
- [ ] Reconcile every partial code change against `api-contract.md`.
- [ ] Commit or remove each partial slice; leave no orphan scratch code.

## T0: Schema And Ownership Contract

- [ ] Define canonical profile schema/profile file shape.
- [ ] Define canonical `settings.toml` UI-settings-only shape.
- [ ] Define canonical corp overlay shape.
- [ ] Define profile id and VM immutable profile assignment semantics.
- [ ] Define default rules location/grouping in profile contract.
- [ ] Define default rule override/mutation semantics.
- [ ] Define plugin config in profile/corp contract.
- [ ] Define credential broker profile contract, including BLAKE3 hash exposure
  and OTel/status counters.
- [ ] Add contract tests proving settings cannot own profile/VM behavior.
- [ ] Add contract tests proving profile owns availability, name, description,
  icon/SVG, assets, rules, MCP, skills, credentials, and VM defaults.
- [ ] Commit T0 with tests.

## T1: Service And Gateway API Routes

- [ ] Add approved service routes:
  - `/profiles/list|create`
  - `/profiles/{profile_id}/info|edit|delete|clone|validate|reload`
  - `/profiles/{profile_id}/assets/info|edit|status|ensure`
  - `/profiles/{profile_id}/enforcement/info|reload|evaluate`
  - `/profiles/{profile_id}/enforcement/rules/list`
  - `/profiles/{profile_id}/enforcement/rules/{rule_id}/edit|delete`
  - `/profiles/{profile_id}/detection/info|reload|evaluate`
  - `/profiles/{profile_id}/detection/rules/list`
  - `/profiles/{profile_id}/detection/rules/{rule_id}/edit|delete`
  - `/profiles/{profile_id}/plugins/info|list`
  - `/profiles/{profile_id}/plugins/{plugin_id}/info|edit`
  - `/profiles/{profile_id}/mcp/info`
  - `/profiles/{profile_id}/mcp/servers/list`
  - `/profiles/{profile_id}/mcp/servers/{server_id}/...`
  - `/profiles/{profile_id}/skills/info|list|add`
  - `/profiles/{profile_id}/skills/{skill_id}/edit|delete`
  - `/profiles/{profile_id}/credentials/info|status|list|reload`
  - `/profiles/{profile_id}/credentials/{credential_id}/info|delete`
- [ ] Add approved VM routes:
  - `/vms/list|create`
  - `/vms/{vm_id}/info|status|edit|delete`
  - `/vms/{vm_id}/start|resume|pause|stop|restart|save|fork|reload-profile`
  - `/vms/{vm_id}/save/status`
  - `/vms/{vm_id}/fork/status`
- [ ] Add approved corp routes:
  - `/corp/info|edit|validate|reload`
- [ ] Add approved settings routes:
  - `/settings/info|edit`
- [ ] Add approved runtime ledger routes:
  - `/security/latest|status`
  - `/enforcement/latest|status`
  - `/detection/latest|status`
  - VM/profile filtered `latest` routes.
- [ ] Make gateway expose the exact same route contract as service.
- [ ] Add route conformance tests for HTTP/UDS parity.
- [ ] Burn old global authoring routes; do not leave compatibility aliases.
- [ ] Add adversarial regression tests proving old global authoring routes fail:
  `/enforcements/list`, `/plugins/global/*`, `/mcp/policy`, `/mcp/tools`.
- [ ] Add adversarial tests for wrong profile ids, wrong VM ids, malformed
  rule ids, invalid enum values, and attempts to mutate immutable VM profile id.
- [ ] Commit T1 with tests.

## T2: Security Rail Burn-Down

- [ ] Remove MCP decision provider behavior.
- [ ] Remove or neutralize `McpPolicy` allow/ask/block evaluation.
- [ ] Move MCP server/tool/resource/prompt decisions to profile rules.
- [ ] Remove NetworkPolicy allow/block decision behavior from security path.
- [ ] Keep network mechanics in network engine: parsing, capture, routing,
  DNS/proxy mechanics, ports, caching, decompression, provider metadata.
- [ ] Ensure HTTP/DNS/domain decisions evaluate through `SecurityRuleSet`.
- [ ] Ensure model/file/process/credential/snapshot decisions evaluate through
  `SecurityRuleSet`.
- [ ] Add tests proving defaults execute after specific corp/profile/user rules.
- [ ] Add tests proving default catch-alls cover non-matching events.
- [ ] Add tests proving mutating defaults changes evaluation behavior.
- [ ] Add tests proving MCP and network old policy engines cannot issue final
  security decisions.
- [ ] Add adversarial tests proving MCP/network mechanics cannot bypass CEL
  enforcement, including malformed MCP tool ids, unknown DNS/HTTP domains, and
  conflicting default/specific rules.
- [ ] Commit T2 with tests.

## T3: Profile/Settings/Corp UI/API Split

- [ ] Remove VM/security/MCP/plugin/credential/profile behavior from settings
  store and settings endpoints.
- [ ] Keep `settings.toml` for UI/app preferences only.
- [ ] Create profile API client/store backed by profile endpoints.
- [ ] Create corp API client/store backed by corp endpoints.
- [ ] Ensure one UI editor surface writes one backing contract only.
- [ ] Allow read-only dashboards to compose sources only with explicit source
  labels.
- [ ] Add frontend tests proving profile text/name/description/icon/rule/plugin
  copy comes from API fixtures, not hard-coded UI copy.
- [ ] Add frontend tests proving enum fields use enum controls and boolean fields
  use boolean controls for direct editors, while preview widgets round-trip
  through contract fields.
- [ ] Add adversarial frontend/API tests proving mixed editor submissions cannot
  write settings/profile/corp in one request.
- [ ] Commit T3 with tests.

## T4: MCP, Plugins, Credentials, Skills UI

- [ ] Replace global MCP tools/policy UI with profile -> server -> tools/resources/prompts.
- [ ] Plugin UI reads profile plugin metadata and edits enable/disable, mode,
  and detection logging level through profile endpoints.
- [ ] Credential UI lists brokered credential refs and BLAKE3 hashes only.
- [ ] Credential status UI shows broker counters from endpoint/OTel-derived
  status.
- [ ] Skill UI can add/edit/remove profile skills through profile endpoints.
- [ ] Ensure no provider API object remains in UI for 1.3.
- [ ] Add adversarial tests for plugin disable/enable invalid modes, invalid
  detection levels, cross-profile MCP tool mutation, and credential secret
  leakage attempts.
- [ ] Commit T4 with tests.

## T5: VM Lifecycle, Assets, Install

- [ ] Normalize VM lifecycle API and frontend calls around `/vms/{vm_id}/...`.
- [ ] Ensure VM assigned profile id is immutable.
- [ ] Implement/verify `pause`, `resume`, `save`, `fork`, and operation status.
- [ ] Ensure profile asset selection is profile-backed.
- [ ] Ensure service asset cache status remains service-runtime only.
- [ ] Re-check install flow no longer depends on dead `capsem setup` assumptions.
- [ ] Verify package UI waits for service readiness and reports install/service
  failures cleanly.
- [ ] Verify assets status surfaces missing `vmlinuz`, `initrd.img`, and rootfs
  accurately.
- [ ] Add adversarial lifecycle/install tests for start-before-assets,
  service-down UI, immutable profile mutation, save/fork failure status, and
  missing initrd/rootfs reporting.
- [ ] Commit T5 with tests.

## T6: Documentation, Changelog, Skills

- [ ] Update architecture docs for profile/settings/corp ownership.
- [ ] Update endpoint/API docs from `api-contract.md`.
- [ ] Update security/rules docs for single CEL/security-rule rail and defaults.
- [ ] Update plugin docs and plugin pages.
- [ ] Update MCP docs: config/discovery mechanics only, decisions are rules.
- [ ] Update credential broker docs, including BLAKE3 hash logging and no secret
  exposure.
- [ ] Update install docs and release notes.
- [ ] Update benchmark docs/page with current 1.3 numbers and EROFS/LZ4HC/zstd
  notes.
- [ ] Update all relevant skills that still describe old settings/profile/API
  behavior.
- [ ] Update changelog only for behavior that is actually implemented and tested.
- [ ] Commit T6 docs/changelog.

## T7: Release Verification Gate

- [ ] Rust focused tests for profile/security/default/plugin/credential contracts.
- [ ] Rust service/gateway route conformance tests.
- [ ] Frontend unit/typecheck tests.
- [ ] Adversarial test suite for old endpoints, invalid schemas, invalid enum
  verbs, profile/settings crossover attempts, and security bypass attempts.
- [ ] Session DB/ledger tests proving detection/enforcement/latest/status expose
  DB-backed truth and include rule/effect/detection data.
- [ ] Sigma parser gate with Python parser.
- [ ] Full smoke cycle.
- [ ] Full `just test` or documented equivalent release test suite.
- [ ] Full install cycle:
  - clean install,
  - service start,
  - UI opens after service readiness,
  - terminal works,
  - assets status/ensure works,
  - package UI failure states are visible.
- [ ] Manual UI sanity pass for settings/profile/policy/plugins/MCP/credentials.
- [ ] Benchmark run or explicit note if unchanged:
  - startup,
  - DB write/ledger,
  - network/MCP path,
  - EROFS/LZ4HC notes.
- [ ] Confirm changelog/docs match implementation.
- [ ] Confirm no dirty release-critical files remain.
- [ ] Final commit or release-prep commit after gates pass.

## Model Breakage Audit

- [x] Audit service routes for profile-less authoring endpoints and ambiguous
  `info`/`status` use.
- [x] Audit gateway forwarding/routes for profile-less authoring endpoints.
- [x] Audit frontend API helpers and UI pages for settings-owned VM behavior.
- [x] Audit config/profile/settings/corp parsing for ownership violations.
- [x] Audit MCP assumptions for global tool/resource/prompt lists.
- [x] Audit credential/provider assumptions for remaining provider API objects.
- [x] Audit VM lifecycle assumptions for immutable profile id, pause/resume/save/fork/status.
- [ ] Audit docs/skills for old endpoint/config mental model.
- [x] Capture initial findings in `model-breakage-audit.md`.

## Release Holds

- [ ] No release until default-rule grouping is contract-tested.
- [ ] No release until profile/settings/corp ownership is codified in docs and code.
- [ ] No release until MCP and network decision ownership violations are removed.
- [ ] No release until UI profile/security/plugin/MCP pages reflect backend
  contract fields without invented config copy.
- [ ] No release until one UI editor surface writes one backing contract.
- [ ] No release until plugin/default profile invariants are tested.
- [ ] No release until frontend Policy/Profile UI is either completed or
  intentionally removed from 1.3.
- [ ] No release until changelog/docs match implemented behavior.
- [ ] No release until smoke, tests, install cycle, and release verification gate pass.

## Commit Discipline

- [x] Contract checkpoint: `9b56f53c docs: define 1.3 profile API contract`.
- [x] UI cardinality checkpoint: `fa212248 docs: codify UI control cardinality`.
- [x] UI widget clarification: `93d6814f docs: clarify UI contract widgets`.
- [x] Profile UI clarification: `8bf798c3 docs: clarify profile UI contract`.
- [x] Settings/profile wording correction: `1e39e5b1 docs: fix settings and profile ownership wording`.
- [x] Mixed editor contract: `9be1503f docs: forbid mixed UI contract editors`.
- [ ] Commit every functional implementation slice with focused tests.
- [ ] Changelog entries land with the behavior-changing commits they describe.

## Coverage Ledger

- Unit/contract: pending.
- Functional API: pending.
- Adversarial: pending.
- E2E/VM: pending.
- Telemetry/session DB: pending.
- Frontend: pending.
- Performance/benchmarks: pending.
- Install/package: pending.
- Docs/changelog: pending.
