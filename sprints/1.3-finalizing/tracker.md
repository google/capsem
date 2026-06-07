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

- [x] Review uncommitted compiler/default-rule changes.
- [x] Review uncommitted service/gateway `/enforcements/list` changes and
  remove in favor of profile-addressed routes.
- [x] Review uncommitted frontend Policy section changes.
- [x] Decide whether to keep, reshape, or remove `sprints/security-default-rule-rail/`.
- [x] Reconcile every partial code change against `api-contract.md`.
- [x] Commit reconciled default-rule rail slice; leave no orphan scratch code.

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
- [x] Burn `/mcp/policy` from service, gateway, CLI, frontend API/store, and
  settings UI. Runtime MCP servers/tools remain as mechanics only.
- [x] Replace plugin authoring routes with profile-scoped
  `/profiles/{profile_id}/plugins/list`,
  `/profiles/{profile_id}/plugins/{plugin_id}/info`, and
  `PATCH /profiles/{profile_id}/plugins/{plugin_id}/edit` in service,
  gateway, and frontend API.
- [x] Add adversarial gateway tests proving retired `/plugins`,
  `/plugins/{vm_id}`, and `/plugins/global/{plugin_id}` routes are not
  forwarded.
- [x] Replace global MCP routes with profile/server-scoped routes in service,
  gateway, frontend API/store, CLI, and capsem-mcp:
  `/profiles/{profile_id}/mcp/servers/list`,
  `/profiles/{profile_id}/mcp/servers/{server_id}/tools/list`,
  `/profiles/{profile_id}/mcp/servers/{server_id}/refresh`,
  `/profiles/{profile_id}/mcp/servers/{server_id}/tools/{tool_id}/edit`, and
  `/profiles/{profile_id}/mcp/servers/{server_id}/tools/{tool_id}/call`.
- [x] Replace global enforcement authoring routes with profile-owned routes:
  `/profiles/{profile_id}/enforcement/evaluate`,
  `PUT /profiles/{profile_id}/enforcement/rules/{rule_id}/edit`,
  `DELETE /profiles/{profile_id}/enforcement/rules/{rule_id}/delete`, and
  `/profiles/{profile_id}/enforcement/reload`.
- [x] Replace the retired `/corp-config` mutation route with `PUT /corp/edit`
  in service and gateway, with regression tests proving the old route is not
  forwarded.
- [x] Replace ambiguous `GET|POST /settings` with `GET /settings/info` and
  `PATCH /settings/edit` in service, gateway, and frontend API, with
  regression tests proving the old route is removed.
- [x] Remove retired settings utility routes `/settings/lint` and
  `/settings/validate-key` from service, gateway, and frontend API, with
  regression tests proving both routes are removed.
- [x] Replace global `POST /reload-config` with
  `POST /profiles/{profile_id}/reload` in service, gateway, frontend API, and
  tests, with regression tests proving the old global route is removed.
- [x] Replace VM ledger routes with
  `/vms/{vm_id}/security|detection|enforcement/latest|status` in service and
  gateway, with regression tests proving retired `/security/{id}`,
  `/detections/{id}`, and `/enforcements/{id}` ledger routes are removed.
- [ ] Add adversarial tests for wrong profile ids, wrong VM ids, malformed
  rule ids, invalid enum values, and attempts to mutate immutable VM profile id.
- [ ] Commit T1 with tests.

## T2: Security Rail Burn-Down

- [ ] Remove MCP decision provider behavior.
- [x] Remove or neutralize `McpPolicy` allow/ask/block evaluation.
- [ ] Move MCP server/tool/resource/prompt decisions to profile rules.
- [x] Remove NetworkPolicy allow/block decision behavior from security path.
- [x] Keep network mechanics in network engine: parsing, capture, routing,
  DNS/proxy mechanics, ports, caching, decompression, provider metadata.
- [x] Remove `PolicyRule`, `NetworkPolicy.rules`,
  `NetworkPolicy.default_allow_read`, and `NetworkPolicy.default_allow_write`
  so network mechanics cannot carry hidden domain decisions.
- [x] Stop exporting retired `CAPSEM_WEB_ALLOW_READ` /
  `CAPSEM_WEB_ALLOW_WRITE` guest env vars from settings.
- [x] Burn retired web decision setting ids from defaults, presets, builder
  schema/model/validation, generated defaults, frontend settings fixtures, and
  checked-in integration fixtures. `security.web` now carries network mechanics
  only (`http_upstream_ports`).
- [ ] Ensure HTTP/DNS/domain decisions evaluate through `SecurityRuleSet`.
- [ ] Ensure model/file/process/credential/snapshot decisions evaluate through
  `SecurityRuleSet`.
- [ ] Add tests proving defaults execute after specific corp/profile/user rules.
- [ ] Add tests proving default catch-alls cover non-matching events.
- [ ] Add tests proving mutating defaults changes evaluation behavior.
- [x] Add tests proving MCP and network old policy engines cannot issue final
  security decisions.
- [x] Burn `McpPolicy`/`ToolDecision`, remove preset MCP permissions, reject
  retired MCP policy config keys, and convert MCP blocking fixture to
  `[profiles.rules.*]`.
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

- [x] Replace global MCP tools/policy UI with profile -> server -> tools for
  the current 1.3 surface. Resources/prompts remain a follow-up endpoint/UI
  gap.
- [x] Plugin UI reads profile plugin metadata and edits enable/disable, mode,
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

## T6.5: Full Invariant Review Before Verification

Before T7, do a fresh full-codebase review against every master contract
invariant. This is not a substitute for tests; it is the final deliberate
invariant sweep before release verification.

### Burn/Compatibility Invariants

- [ ] No old policy-v2 paths are live.
- [ ] No old authoring API fallback routes remain.
- [ ] No old authoring API compatibility aliases remain.
- [ ] No runtime branch accepts both old and new ownership models.
- [ ] No `if old shape then...` escape hatch remains.
- [ ] Dead policy/API/config code is removed, not quarantined.
- [ ] Tests prove old paths/shapes fail closed.

### Architecture Ownership Invariants

- [ ] No `NetworkRouting` abstraction was added.
- [ ] Network engine owns mechanics only: parsing, capture, DNS/proxy mechanics,
  ports, caching, decompression, routing mechanics, provider metadata.
- [ ] Network engine does not own security decisions.
- [ ] MCP owns config/discovery mechanics only: servers, tools, resources,
  prompts, runtime discovery/status.
- [ ] MCP does not own security decisions.
- [ ] Service-global endpoints only report runtime/service/ledger state.

### Security Rail Invariants

- [ ] All allow/ask/block/rewrite/preprocess/postprocess decisions are
  CEL/security-rule decisions over typed security events.
- [ ] HTTP decisions use the security rule rail.
- [ ] DNS decisions use the security rule rail.
- [ ] MCP decisions use the security rule rail.
- [ ] Model decisions use the security rule rail.
- [ ] File decisions use the security rule rail.
- [ ] Process decisions use the security rule rail.
- [ ] Credential decisions/effects use the security rule/plugin rail.
- [ ] Snapshot decisions use the security rule rail.
- [ ] Default rules are visible real rules in the same `SecurityRuleSet`.
- [ ] There is no second default engine.
- [ ] `priority = "default"` is the only post-user catch-all sentinel.
- [ ] Specific corp/profile/user rules evaluate before defaults.
- [ ] Plugins expose explicit event effects and do not hide a second policy
  engine.
- [ ] Block decisions are absolute.
- [ ] Runtime ledger endpoints report stored DB truth, not recomputed active
  policy state.

### Profile/Settings/Corp Invariants

- [ ] A VM executes exactly one immutable profile id.
- [ ] VM profile id cannot be edited.
- [ ] Profile owns assets.
- [ ] Profile owns VM config/defaults.
- [ ] Profile owns rules/enforcement defaults.
- [ ] Profile owns detection rules.
- [ ] Profile owns MCP config.
- [ ] Profile owns skills.
- [ ] Profile owns credentials/plugins.
- [ ] Profile owns availability.
- [ ] Profile owns name, description, and icon/SVG.
- [ ] `settings.toml` owns UI/application preferences only.
- [ ] Settings do not own VM behavior.
- [ ] Settings do not own security rules.
- [ ] Settings do not own MCP config.
- [ ] Settings do not own plugin config.
- [ ] Settings do not own credentials.
- [ ] Settings do not own profile identity or availability.
- [ ] Corp owns constraints, locks, reporting, and integrations over profiles.

### Endpoint/DTO Invariants

- [ ] HTTP and UDS expose the same route contract.
- [ ] HTTP and UDS expose the same DTO contract.
- [ ] HTTP and UDS expose the same error contract.
- [ ] `info` endpoints return configuration/metadata only.
- [ ] `status` endpoints return runtime state/counters/readiness/progress.
- [ ] `latest` endpoints return DB-backed ledger rows.
- [ ] `list` endpoints return child collections.
- [ ] `edit` endpoints mutate one backing contract.
- [ ] `reload` endpoints re-read/apply owned config files.
- [ ] No generic `rule-files` API exists.
- [ ] Enforcement source refs are exposed through enforcement `info`.
- [ ] Detection source refs are exposed through detection `info`.
- [ ] Provider is not a 1.3 profile API object.
- [ ] Credential brokerage plus rules own provider-like behavior.

### UI Invariants

- [ ] One UI editor surface writes one backing contract.
- [ ] Settings UI writes only settings-backed data.
- [ ] Profile UI writes only profile-backed data.
- [ ] Corp UI writes only corp-backed data.
- [ ] Runtime/ledger UI is read-only unless it calls explicit runtime action
  endpoints.
- [ ] Cross-source dashboards are read-only and label source data.
- [ ] UI does not rename backend-owned objects.
- [ ] UI does not invent explanatory config text.
- [ ] Rule names/reasons/actions/groups/sources come from backend fields.
- [ ] Plugin names/descriptions come from backend fields and docs links.
- [ ] MCP server/tool/resource/prompt names come from backend fields.
- [ ] Skill names/descriptions come from backend fields.
- [ ] Credential ids/hashes come from backend fields.
- [ ] Asset names/status come from backend fields.
- [ ] Direct boolean editors use boolean controls.
- [ ] Direct enum editors use enum controls.
- [ ] Direct numeric editors use numeric controls with backend constraints.
- [ ] Rich preview/composed widgets round-trip through the same contract fields.

### Install/Release Invariants

- [ ] Install flow does not depend on dead setup assumptions.
- [ ] Package UI waits for service readiness.
- [ ] Package UI reports service/install failures visibly.
- [ ] Asset status reports missing `vmlinuz`, `initrd.img`, and rootfs
  accurately.
- [ ] Changelog matches implemented behavior only.
- [ ] Docs and skills match implemented behavior only.
- [ ] Benchmark docs include current 1.3 performance notes or explicitly state
  what was not rerun.
- [ ] Commit T6.5 invariant review findings/fixes before T7.

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
- [x] Default-rule implementation checkpoint: `e283c711 feat: make security defaults explicit rules`.
- [ ] Commit every functional implementation slice with focused tests.
- [ ] Changelog entries land with the behavior-changing commits they describe.

## Coverage Ledger

- Unit/contract: `cargo test -p capsem-core net::policy_config::security_rule_profile --lib`; `cargo test -p capsem-core net::policy_config::provider_profile --lib`; `cargo test -p capsem-core net::policy_config --lib`; `cargo test -p capsem-core mcp:: --lib`; `cargo test -p capsem-core net::policy --lib`; `cargo test -p capsem-core net::dns::cache --lib`; `cargo test -p capsem-core net::dns --lib`; `uv run python -m pytest tests/test_models.py tests/test_config.py tests/test_validate.py tests/test_cli.py -q`.
- Functional API: `cargo check -p capsem-service -p capsem-gateway -p capsem -p capsem-mcp`; `cargo check -p capsem-service -p capsem-gateway`; `cargo build -p capsem-service`; `uv run python -m pytest tests/capsem-service/test_svc_mcp_api.py -q`; `uv run python -m pytest tests/capsem-service/test_svc_install.py -q`; `uv run python -m pytest tests/capsem-service/test_svc_settings.py tests/capsem-service/test_svc_install.py -q`; `uv run python -m pytest tests/capsem-service/test_svc_core.py tests/capsem-service/test_svc_settings.py -q`; `uv run python -m pytest tests/capsem-service/test_svc_settings.py -q`; `uv run python -m pytest tests/capsem-gateway/test_gw_proxy_advanced.py -q`; `cargo test -p capsem-gateway --bin capsem-gateway gateway_`; `cargo test -p capsem-service --bin capsem-service handle_`; `cargo test -p capsem-service --bin capsem-service security_latest_returns_full_session_db_rule_ledger_rows`; `cargo test -p capsem-service --bin capsem-service profile_plugin_endpoint_matrix_dynamically_controls_enforcement_evaluation`; `cargo test -p capsem-service --bin capsem-service enforcement_rule_endpoints_add_delete_reload_and_reject_invalid_rules_atomically`.
- Adversarial: `/mcp/policy` and retired global `/mcp/servers`, `/mcp/tools`, `/mcp/tools/refresh`, `/mcp/tools/{name}/approve`, and `/mcp/tools/{name}/call` are removed from service/gateway routes, with `tests/capsem-service/test_svc_mcp_api.py::TestMcpPolicy::test_retired_mcp_endpoints_are_burned` and `cargo test -p capsem-gateway gateway_`; retired `/plugins`, `/plugins/{vm_id}`, and `/plugins/global/{plugin_id}` are not forwarded by gateway; retired global enforcement authoring routes `/enforcements/evaluate`, `/enforcements/rules/{rule_id}`, and `/enforcements/reload` are not forwarded by gateway; retired `/security/{id}/latest|info`, `/detections/{id}/latest|info`, and `/enforcements/{id}/latest|info` are not forwarded by gateway; retired `/corp-config` is rejected by service and not forwarded by gateway; retired `GET|POST /settings` is rejected by service and not forwarded by gateway; retired `/settings/lint` and `/settings/validate-key` are rejected by service and not forwarded by gateway; retired `POST /reload-config` is rejected by service and not forwarded by gateway; retired `mcp.global_policy`, `mcp.default_tool_permission`, and `mcp.tool_permissions` rejected by `load_settings_file_rejects_retired_mcp_policy_keys`; `rg -n "NetworkPolicy::evaluate|\\.evaluate\\(\\\"|is_fully_blocked|PolicyDecision|read allowed by default|write denied by default|fully blocked|blocked domain stays NXDOMAIN" crates/capsem-core/src/net crates/capsem-core/src/net/policy_config/tests.rs -g '*.rs'` returned no matches after burning network allow/block APIs; `rg -n "PolicyRule|NetworkPolicy::evaluate|PolicyDecision|is_fully_blocked|default_allow_read|default_allow_write|network\\.rules|allow_read|allow_write" crates/capsem-core/src crates/capsem-core/tests crates/capsem-service/src crates/capsem-gateway/src -g '*.rs'` has no active domain-decision type/field hits outside retired setting ids/tests; `web_default_toggles_not_exposed_as_guest_authority` proves stale web toggles do not produce guest env authority; `batch_update_rejects_retired_web_decision_setting_ids`, `migrate_setting_ids_does_not_resurrect_retired_web_decision_keys`, `retired_web_decision_settings_are_not_resolved`, and Python `TestRetiredWebDecisionConfig::test_allow_block_fields_fail_closed` prove retired web decision settings fail closed or remain inert stale input.
- E2E/VM: pending.
- Telemetry/session DB: pending.
- Frontend: `pnpm --dir frontend test src/lib/__tests__/api.test.ts src/lib/__tests__/mcp-store.test.ts`; `pnpm --dir frontend test src/lib/__tests__/api.test.ts`; `pnpm --dir frontend test src/lib/__tests__/settings-store.test.ts src/lib/models/__tests__/settings-model.test.ts`; `pnpm --dir frontend check`; `api.test.ts` proves settings calls `GET /settings/info` and `PATCH /settings/edit`, reload calls `POST /profiles/default/reload`, plugin API calls profile-scoped plugin routes and uses `PATCH`, MCP API calls profile/server-scoped routes, and no settings lint helper remains.
- Performance/benchmarks: pending.
- Install/package: pending.
- Docs/changelog: `CHANGELOG.md` updated for the MCP policy API/UI/CLI burn, retired web decision settings burn, profile-scoped plugin API, profile/server-scoped MCP API, profile-owned enforcement authoring API, `/corp/edit` replacement for retired `/corp-config`, `/settings/info|edit` replacement for retired magic `/settings`, removal of retired `/settings/lint` and `/settings/validate-key`, profile reload replacement for retired `/reload-config`, and VM-scoped ledger route replacement for retired `/security|detections|enforcements/{id}` routes.
