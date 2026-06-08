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
- [x] Audit profile/platform lost work and capture it in
  `profile-platform-lost-work-audit.md`.

## Current Partial Work To Reconcile

- [x] Review uncommitted compiler/default-rule changes.
- [x] Review uncommitted service/gateway `/enforcements/list` changes and
  remove in favor of profile-addressed routes.
- [x] Review uncommitted frontend Policy section changes.
- [x] Decide whether to keep, reshape, or remove `sprints/security-default-rule-rail/`.
- [x] Reconcile every partial code change against `api-contract.md`.
- [x] Commit reconciled default-rule rail slice; leave no orphan scratch code.

## T0: Schema And Ownership Contract

- [x] Define canonical profile schema/profile file shape.
- [x] Define canonical `settings.toml` UI-settings-only shape.
- [x] Define canonical corp overlay shape.
- [x] Define profile id and VM immutable profile assignment semantics.
- [x] Define default rules location/grouping in profile contract.
- [x] Define default rule override/mutation semantics.
- [x] Define plugin config in profile/corp contract.
- [x] Define credential broker plugin runtime contract, including opaque
  BLAKE3 hash exposure and OTel/status counters.
- [x] Add contract tests proving settings cannot own profile/VM behavior.
- [x] Add contract tests proving profile owns availability, name, description,
  icon/SVG, assets, rules, MCP, skills, plugin config, and VM defaults.
- [x] Commit T0 with tests.

### T0 Notes

- Added `policy_config::ownership` with public validators for
  `settings.toml`, `profile.toml`, and `corp.toml` ownership.
- `settings.toml` accepts only `app.*` and `appearance.*` UI/application
  preferences and rejects profile behavior sections (`rule_files`,
  `profiles`, `corp`, `ai`, `plugins`, tool config sources, MCP).
- Profile-owned config writes now use
  `batch_update_profile_settings*`; `/settings/edit` keeps
  `batch_update_settings*` and rejects VM/security/AI/repository/credential
  settings.
- `cargo test -p capsem-core ownership::tests` passed with 6 ownership
  contract tests.
- `cargo test -p capsem-core profile_contract::tests` passed with 4 profile
  manifest contract tests covering identity, description, icon SVG,
  availability, EROFS assets, VM defaults, rules/defaults, AI/provider rules,
  plugins, MCP, skills, and tool config sources.
- `cargo test -p capsem-core batch_update` passed with 11 batch-writer
  ownership/atomicity tests.
- `cargo clippy -p capsem-core --all-targets -- -D warnings` passed.

## T1: Service And Gateway API Routes

- [x] Add approved service routes:
  - `[x] /profiles/list`
  - `[x] /profiles/create`
  - `[x] /profiles/{profile_id}/info`
  - `[x] /profiles/{profile_id}/edit|delete|clone|validate`
  - `[x] /profiles/{profile_id}/reload`
  - `[x] /profiles/{profile_id}/assets/info|edit`
  - `[x] /profiles/{profile_id}/assets/status|ensure`
  - `[x] /profiles/{profile_id}/enforcement/info|reload|evaluate`
  - `[x] /profiles/{profile_id}/enforcement/rules/list`
  - `[x] /profiles/{profile_id}/enforcement/rules/{rule_id}/edit|delete`
  - `[x] /profiles/{profile_id}/detection/info|reload|evaluate`
  - `[x] /profiles/{profile_id}/detection/rules/list`
  - `[x] /profiles/{profile_id}/detection/rules/{rule_id}/edit|delete`
  - `[x] /profiles/{profile_id}/plugins/info|list`
  - `[x] /profiles/{profile_id}/plugins/{plugin_id}/info|edit`
  - `[x] /profiles/{profile_id}/mcp/info`
  - `[x] /profiles/{profile_id}/mcp/servers/list`
  - `[x] /profiles/{profile_id}/mcp/servers/{server_id}/...`
  - `[x] /profiles/{profile_id}/skills/info|list|add`
  - `[x] /profiles/{profile_id}/skills/{skill_id}/edit|delete`
- [x] Add approved VM routes:
  - `[x] /vms/list|create`
  - `[x] /vms/{vm_id}/info|status|edit|delete`
  - `[x] /vms/{vm_id}/start|resume|pause|stop|restart|save|fork|reload-profile`
  - `[x] /vms/{vm_id}/save/status`
  - `[x] /vms/{vm_id}/fork/status`
- [x] Add approved corp routes:
  - `[x] /corp/info|edit|validate|reload`
- [x] Add approved settings routes:
  - `[x] /settings/info|edit`
- [x] Add approved runtime ledger routes:
  - `[x] /security/latest|status`
  - `[x] /enforcement/latest|status`
  - `[x] /detection/latest|status`
  - `[x] VM/profile filtered latest routes`
- [x] Make gateway expose the exact same route contract as service.
- [x] Add route conformance tests for HTTP/UDS parity.
- [x] Burn old global authoring routes; do not leave compatibility aliases.
- [x] Add adversarial regression tests proving old global authoring routes fail:
  `/enforcements/list`, `/plugins/global/*`, `/mcp/policy`, `/mcp/tools`.
- [x] Burn `/mcp/policy` from service, gateway, CLI, frontend API/store, and
  settings UI. Runtime MCP servers/tools remain as mechanics only.
- [x] Replace plugin authoring routes with profile-scoped
  `/profiles/{profile_id}/plugins/list`,
  `/profiles/{profile_id}/plugins/{plugin_id}/info`, and
  `PATCH /profiles/{profile_id}/plugins/{plugin_id}/edit` in service,
  gateway, and frontend API.
- [x] Add profile inventory routes in service, gateway, and frontend API:
  `GET /profiles/list` and `GET /profiles/{profile_id}/info`. The built-in
  `default` summary is now sourced from `ProfileConfigFile::builtin_default()`;
  fake profile IDs fail closed while independent profile file loading remains
  a later route slice.
- [x] Add profile create/edit/delete/clone/validate routes in service, gateway,
  and frontend API. `validate` checks the typed `ProfileConfigFile` contract;
  mutation routes fail explicitly with `501` until profile file persistence
  exists.
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
- [x] Add profile-owned enforcement rule inventory:
  `GET /profiles/{profile_id}/enforcement/rules/list` in service, gateway, and
  frontend API. The response is compiled rule truth with source/default/priority
  metadata, and fake profile IDs fail closed.
- [x] Add profile-owned enforcement info:
  `GET /profiles/{profile_id}/enforcement/info` in service, gateway, and
  frontend API. The response summarizes the same compiled rule inventory and
  fake profile IDs fail closed.
- [x] Add profile-owned detection rule routes in service, gateway, and
  frontend API. Detection routes reuse the enforcement rule DTO/engine, filter
  inventory to rules with `detection_level`, and reject detection writes that
  would not emit a detection.
- [x] Replace global asset status/ensure routes with profile-owned
  `/profiles/{profile_id}/assets/status` and
  `/profiles/{profile_id}/assets/ensure` in service, gateway, frontend API,
  CLI, and service integration tests. Old global asset routes fail closed.
- [x] Add profile-owned skills routes in service, gateway, and frontend API.
  Credential profile routes were later burned; credential broker state is
  plugin-owned runtime status/stats.
- [x] Add profile-owned assets info/edit, plugins info, and MCP info routes in
  service, gateway, and frontend API. Info routes summarize typed profile/config
  state; asset edits fail explicitly until profile persistence lands.
- [x] Add service-wide runtime ledger routes in service, gateway, and frontend
  API. Routes aggregate session DB rows through `DbReader`; detection filters to
  rows with non-`none` detection level.
- [x] Replace the retired `/corp-config` mutation route with `PUT /corp/edit`
  in service and gateway, with regression tests proving the old route is not
  forwarded.
- [x] Add approved `/corp/info`, `/corp/validate`, and `/corp/reload` routes
  in service and gateway.
- [x] Replace ambiguous `GET|POST /settings` with `GET /settings/info` and
  `PATCH /settings/edit` in service, gateway, and frontend API, with
  regression tests proving the old route is removed.
- [x] Remove retired settings utility routes `/settings/lint` and
  `/settings/validate-key` from service, gateway, and frontend API, with
  regression tests proving both routes are removed.
- [x] Remove retired settings preset routes and UI selector from service,
  gateway, and frontend, with regression tests proving `/settings/presets` no
  longer exists.
- [x] Remove preset metadata from the settings response/model so settings
  carries UI/app preferences only.
- [x] Replace global `POST /reload-config` with
  `POST /profiles/{profile_id}/reload` in service, gateway, frontend API, and
  tests, with regression tests proving the old global route is removed.
- [x] Replace VM ledger routes with
  `/vms/{vm_id}/security|detection|enforcement/latest|status` in service and
  gateway, with regression tests proving retired `/security/{id}`,
  `/detections/{id}`, and `/enforcements/{id}` ledger routes are removed.
- [x] Replace retired top-level VM lifecycle routes with
  `/vms/{vm_id}/pause`, `/vms/{vm_id}/delete`,
  `/vms/{vm_id}/resume`, `/vms/{vm_id}/save`, and
  `/vms/{vm_id}/fork` in service, gateway, CLI, MCP, tray, frontend API, and
  tests; gateway regression tests prove old `/suspend`, `/delete`, `/resume`,
  `/persist`, and `/fork` routes are not forwarded.
- [x] Replace core VM routes with `/vms/create`, `/vms/list`,
  `/vms/{vm_id}/info`, and `/vms/{vm_id}/stop` in service, gateway, CLI, MCP,
  tray, frontend API, status aggregation, docs, and tests; gateway regression
  tests prove old `/provision`, `/list`, `/info/{id}`, and `/stop/{id}` routes
  are not forwarded.
- [x] Add `GET /vms/{vm_id}/status` as a runtime-only VM state route in
  service, gateway, frontend API, docs, and tests.
- [x] Add `PATCH /vms/{vm_id}/edit` as a fail-closed VM edit gate in service
  and gateway, with handler tests proving `profile_id` is immutable, unknown
  fields fail, and unsupported resource edits do not silently succeed.
- [x] Add `/vms/{vm_id}/save/status` and `/vms/{vm_id}/fork/status` in service
  and gateway, with handler tests proving existing VMs report explicit
  synchronous `idle` operation state and unknown VMs fail closed.
- [x] Add `/vms/{vm_id}/start`, `/vms/{vm_id}/restart`, and
  `/vms/{vm_id}/reload-profile` routes in service and gateway. `start` uses
  the existing resume/start path; restart and reload-profile fail explicitly
  with handler tests until real semantics are implemented.
- [x] Replace VM utility routes with `/vms/{vm_id}/exec`,
  `/vms/{vm_id}/logs`, `/vms/{vm_id}/inspect`,
  `/vms/{vm_id}/timeline`, `/vms/{vm_id}/history...`, and
  `/vms/{vm_id}/files...` in service, gateway, CLI, MCP, frontend API, docs,
  and tests; gateway regression tests prove old `/exec`, `/logs`, `/inspect`,
  `/timeline`, `/history`, `/read_file`, `/write_file`, and `/files` routes
  are not forwarded.
- [x] Add adversarial tests for wrong profile ids, wrong VM ids, malformed
  rule ids, invalid enum values, and attempts to mutate immutable VM profile id.
- [x] Commit T1 with tests.

## T2: Security Rail Burn-Down

- [x] Remove MCP decision provider behavior.
- [x] Remove or neutralize `McpPolicy` allow/ask/block evaluation.
- [x] Move MCP server/tool/resource/prompt decisions to profile rules.
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
- [x] Ensure HTTP/DNS/domain decisions evaluate through `SecurityRuleSet`.
- [x] Ensure model/file/process decisions evaluate through `SecurityRuleSet`;
  burn fake credential/snapshot rule roots instead of pretending they have
  parsers.
- [x] Burn rule-dispatched plugin behavior. Rules cannot use `plugin = ...`;
  plugins run from typed plugin config, own their own filtering, and execute by
  plugin stage.
- [x] Add fail-closed tests proving configured-but-unregistered plugins do not
  silently disappear.
- [x] Add tests proving defaults execute after specific corp/profile/user rules.
- [x] Add tests proving default catch-alls cover non-matching events.
- [x] Add tests proving mutating defaults changes evaluation behavior.
- [x] Add tests proving MCP and network old policy engines cannot issue final
  security decisions.
- [x] Burn `McpPolicy`/`ToolDecision`, remove preset MCP permissions, reject
  retired MCP policy config keys, and convert MCP blocking fixture to
  `[profiles.rules.*]`.
- [x] Add adversarial tests proving MCP/network mechanics cannot bypass CEL
  enforcement, including malformed MCP tool ids, unknown DNS/HTTP domains, and
  conflicting default/specific rules.
- [x] Commit T2 with tests.

### T2 Notes

- Removed T2 drift from active docs: no user-facing docs now teach
  `allow_read`, `allow_write`, `custom_allow`, `custom_block`, Policy V2,
  MCP decision providers, or domain-policy engines as security authorities.
- `cargo test -p capsem-core security_rule_profile::tests` passed with 26
  rule-profile tests, including default coverage for HTTP, DNS, MCP, model,
  file, and process events.
- `cargo test -p capsem-core --lib security_engine::tests -- --nocapture`
  passed with 38 tests, including plugin stage execution, disabled-plugin skip,
  configured-missing-plugin fail-closed behavior, credential broker observation
  handling, EICAR dummy plugin block proof, absolute block lattice, and ledger
  regeneration.
- `cargo test -p capsem-core --lib provider_profile::tests -- --nocapture`
  passed with 6 provider/default contract tests after broker invocation rules
  were removed.
- `cargo clippy -p capsem-core --all-targets -- -D warnings` passed after the
  `NetworkPolicy: Default` and test assertion clippy fixes.
- `rg -n 'allow_read|allow_write|custom_allow|custom_block|Policy V2|policy_v2|McpPolicy|ToolDecision|DecisionProvider|PolicyHook|is_fully_blocked|default_allow|Domain policy|domain policy|default-deny|default deny|allow list|block list|/enforcements/|/detections/|/plugins/global' docs/src/content/docs -S`
  returned no matches after the docs burn pass.

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
- [x] Make profile MCP service routes read the selected `ProfileConfigFile.mcp`
  instead of settings/corp MCP sections. The `code` profile explicitly enables
  the real built-in `local` MCP server, the profile-only MCP builder avoids
  host AI config auto-detection, and unknown profile server ids fail closed.
  Coverage: `cargo test -p capsem-core mcp::tests::build_profile_server_list --
  --nocapture`, `cargo test -p capsem-core --lib profile_contract --
  --nocapture`, `cargo test -p capsem-service profile_mcp -- --nocapture`,
  `cargo test -p capsem-service --no-run`, `cargo build -p capsem-service`,
  and `uv run pytest tests/capsem-service/test_svc_mcp_api.py -q`.
- [x] Plugin UI reads profile plugin metadata and edits enable/disable, mode,
  and detection logging level through profile endpoints.
- [ ] Credential UI reads only credential-broker plugin runtime status/stats and
  lists brokered refs/BLAKE3 hashes from that plugin-owned state.
- [ ] Skill UI can add/edit/remove profile skills through profile endpoints.
- [x] Ensure no provider API object remains in UI for 1.3. `/settings/info`
  now serializes only `tree` and `issues`, the frontend settings model/store
  have no provider-status accessor, and runtime `top_providers` analytics stay
  separate from configuration. Coverage: `cargo test -p capsem-core --lib
  load_settings_response -- --nocapture`, `cargo test -p capsem-service
  handle_get_settings_returns_tree -- --nocapture`, `pnpm -C frontend test
  src/lib/models/__tests__/settings-model.test.ts
  src/lib/__tests__/settings-store.test.ts`, and `pnpm -C frontend check`.
- [ ] Add adversarial tests for plugin disable/enable invalid modes, invalid
  detection levels, cross-profile MCP tool mutation, and credential secret
  leakage attempts.
- [ ] Commit T4 with tests.

## T5: VM Lifecycle, Assets, Install

- [x] Normalize VM lifecycle API and frontend calls around `/vms/{vm_id}/...`.
- [ ] Execute focused snapshot restore sub-sprint:
  `sprints/1.3-finalizing/snapshot-restore/`.
- [ ] Ensure VM assigned profile id is immutable.
- [ ] Implement/verify `pause`, `resume`, `save`, `fork`, and operation status.
- [ ] Restore profile catalog/loader and remove the current `default`-only
  route validator.
- [x] Add the first catalog-backed profile route slice: core parses
  `config/profiles/code.toml` with per-arch EROFS/LZ4HC assets, and service
  profile route validation/list/info/assets/skills/plugin checks use catalog
  lookup for `code` instead of a hard-coded `default` stub.
- [x] Make profile asset status profile-aware: status reports the selected
  profile's current-arch asset metadata and present/missing state instead of a
  service-global asset guess.
- [ ] Ensure profile asset selection is profile-backed:
  `vm.profile_id -> profile assets -> asset manifest/cache -> resolved boot paths`.
- [ ] Restore per-arch profile asset declarations with URL/hash/signature/size
  metadata.
- [ ] Restore profile-aware asset reconciliation/status/ensure.
- [ ] Restore persistent VM profile/base-asset pins and fail-closed resume/fork/save.
- [ ] Restore VM/profile DTOs for profile id, revision, status, pin, and base assets.
- [ ] Restore TUI crate and terminal shell behavior; `capsem shell` must work
  through the TUI again.
- [ ] Restore launchable-profile filtering for UI/TUI/gateway.
- [ ] Reconcile release/CI profile asset generation so package profiles point at
  release EROFS/lz4hc assets.
- [ ] Restore `capsem-admin` as the typed profile/settings/asset/manifest/security
  pack command surface used by `just`, CI, package payloads, and release gates.
- [ ] Restore `scripts/build-assets.sh --profile <profile>` or an equivalent
  `just build-assets profile=...` path that delegates profile-derived
  kernel/rootfs builds through `capsem-admin`, not raw shell state.
- [ ] Restore package/bootstrap proof that `capsem-admin` is installed and
  runnable from native packages.
- [ ] Restore admin manifest crypto/generate/download-check gates before release.
- [ ] Classify every `82e7a58c^1..82e7a58c` deleted cluster as intentional
  burn, conceptual port, or exact restore before closing T5.
- [ ] Restore or Linux-team handoff the KVM/checkpoint, EROFS/LZ4HC, multi-arch,
  and benchmark proof trail. Do not close 1.3 with missing Linux evidence unless
  it is an explicit release blocker owned by Linux.
- [ ] Treat Linux-team scoped commits as authoritative in their files; restore
  or port them unless they directly violate the current security/profile
  contract.
- [ ] Restore advanced benchmark harness/artifacts/docs for EROFS/LZ4HC and
  current security-event/CEL performance.
- [ ] Restore security pack/detection/backtest/corpus gates on the new
  `SecurityRuleSet`/CEL rail.
- [ ] Review debug/status diagnostics for survivable loss; restore only if
  needed for install/support proof.
- [ ] Ensure service asset cache status remains service-runtime only.
- [ ] Re-check install flow no longer depends on dead `capsem setup` assumptions.
- [ ] Verify package UI waits for service readiness and reports install/service
  failures cleanly.
- [ ] Verify assets status surfaces missing `vmlinuz`, `initrd.img`, and rootfs
  accurately.
- [ ] Add adversarial lifecycle/install tests for start-before-assets,
  service-down UI, immutable profile mutation, fake profile ids, two profiles
  with different assets, missing/corrupt profile assets, missing profile pins,
  save/fork failure status, and missing initrd/rootfs reporting.
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
- [ ] Profile owns asset release/logical selection before the asset manifest
  resolves hashes/paths.
- [ ] Persistent VMs store profile and base-asset pins.
- [ ] Resume/fork/save fail closed when profile or base-asset pins are missing.
- [ ] Profile owns VM config/defaults.
- [ ] Profile owns rules/enforcement defaults.
- [ ] Profile owns detection rules.
- [ ] Profile owns MCP config.
- [ ] Profile owns skills.
- [ ] Profile owns plugin config; credential broker secrets/state are plugin
  runtime state.
- [ ] Profile owns availability.
- [ ] Profile owns name, description, and icon/SVG.
- [ ] `settings.toml` owns UI/application preferences only.
- [ ] Settings do not own VM behavior.
- [ ] Settings do not own security rules.
- [ ] Settings do not own MCP config.
- [ ] Settings do not own plugin config.
- [ ] Settings do not own credential broker config/state.
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
- [x] Provider is not a 1.3 profile/settings API object.
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
- [ ] Brokered credential hashes/status come from plugin runtime fields.
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
- [ ] Manual UI sanity pass for settings/profile/policy/plugins/MCP and
  credential broker plugin status.
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

- Unit/contract: `cargo test -p capsem-core net::policy_config::security_rule_profile --lib`; `cargo test -p capsem-core net::policy_config::provider_profile --lib`; `cargo test -p capsem-core net::policy_config --lib`; `cargo test -p capsem-core mcp:: --lib`; `cargo test -p capsem-core net::policy --lib`; `cargo test -p capsem-core net::dns::cache --lib`; `cargo test -p capsem-core net::dns --lib`; `uv run python -m pytest tests/test_models.py tests/test_config.py tests/test_validate.py tests/test_cli.py -q`; `cargo test -p capsem-gateway --bin capsem-gateway gateway_`; `cargo test -p capsem-mcp`; `pnpm --dir frontend test src/lib/__tests__/api.test.ts`; `cargo test -p capsem-service --bin capsem-service handle_vm_edit`; `cargo test -p capsem-service --bin capsem-service handle_vm_operation_status`; `cargo test -p capsem-service --bin capsem-service handle_unsupported_vm_operations`.
- Functional API: `cargo check -p capsem-service -p capsem-gateway -p capsem -p capsem-mcp`; `cargo check -p capsem-service -p capsem-gateway -p capsem -p capsem-mcp -p capsem-tray`; `cargo check -p capsem-core -p capsem-service -p capsem-gateway`; `cargo check -p capsem-service -p capsem-gateway`; `cargo build -p capsem-service`; `uv run python -m pytest tests/capsem-service/test_svc_mcp_api.py -q`; `uv run python -m pytest tests/capsem-service/test_svc_install.py -q`; `uv run python -m pytest tests/capsem-service/test_svc_settings.py tests/capsem-service/test_svc_install.py -q`; `uv run python -m pytest tests/capsem-service/test_svc_core.py tests/capsem-service/test_svc_settings.py -q`; `uv run python -m pytest tests/capsem-service/test_svc_settings.py -q`; `uv run python -m pytest tests/capsem-gateway/test_gw_proxy_advanced.py -q`; `uv run python -m pytest tests/capsem-gateway/test_gw_proxy.py tests/capsem-gateway/test_gw_proxy_advanced.py -q`; `uv run python -m pytest --collect-only tests -q`; `cargo test -p capsem-gateway --bin capsem-gateway gateway_`; `cargo test -p capsem-gateway proxy::tests::returns_502_for_delete_when_uds_missing`; `cargo test -p capsem-gateway proxy::tests::returns_502_for_post_when_uds_missing`; `cargo test -p capsem-mcp`; `cargo test -p capsem-tray gateway`; `cargo test -p capsem-core net::policy_config --lib`; `cargo test -p capsem-service --bin capsem-service handle_`; `cargo test -p capsem-service --bin capsem-service handle_get_settings_returns_tree`; `cargo test -p capsem-service --bin capsem-service security_latest_returns_full_session_db_rule_ledger_rows`; `cargo test -p capsem-service --bin capsem-service profile_plugin_endpoint_matrix_dynamically_controls_enforcement_evaluation`; `cargo test -p capsem-service --bin capsem-service enforcement_rule_endpoints_add_delete_reload_and_reject_invalid_rules_atomically`.
- Adversarial: `/mcp/policy` and retired global `/mcp/servers`, `/mcp/tools`, `/mcp/tools/refresh`, `/mcp/tools/{name}/approve`, and `/mcp/tools/{name}/call` are removed from service/gateway routes, with `tests/capsem-service/test_svc_mcp_api.py::TestMcpPolicy::test_retired_mcp_endpoints_are_burned` and `cargo test -p capsem-gateway gateway_`; retired `/plugins`, `/plugins/{vm_id}`, and `/plugins/global/{plugin_id}` are not forwarded by gateway; retired global enforcement authoring routes `/enforcements/evaluate`, `/enforcements/rules/{rule_id}`, and `/enforcements/reload` are not forwarded by gateway; retired `/security/{id}/latest|info`, `/detections/{id}/latest|info`, and `/enforcements/{id}/latest|info` are not forwarded by gateway; retired `/corp-config` is rejected by service and not forwarded by gateway; retired `GET|POST /settings` is rejected by service and not forwarded by gateway; retired `/settings/presets`, `/settings/lint`, and `/settings/validate-key` are rejected by service and not forwarded by gateway; retired `POST /reload-config` is rejected by service and not forwarded by gateway; retired `/provision`, `/list`, `/info/{id}`, `/stop/{id}`, `/suspend/{id}`, `/delete/{id}`, `/resume/{id}`, `/persist/{id}`, `/fork/{id}`, `/exec/{id}`, `/logs/{id}`, `/inspect/{id}`, `/timeline/{id}`, `/history/{id}`, `/read_file/{id}`, `/write_file/{id}`, `/files/{id}`, and `/files/{id}/content` VM routes are not forwarded by gateway; retired `mcp.global_policy`, `mcp.default_tool_permission`, and `mcp.tool_permissions` rejected by `load_settings_file_rejects_retired_mcp_policy_keys`; `rg -n "NetworkPolicy::evaluate|\\.evaluate\\(\\\"|is_fully_blocked|PolicyDecision|read allowed by default|write denied by default|fully blocked|blocked domain stays NXDOMAIN" crates/capsem-core/src/net crates/capsem-core/src/net/policy_config/tests.rs -g '*.rs'` returned no matches after burning network allow/block APIs; `rg -n "PolicyRule|NetworkPolicy::evaluate|PolicyDecision|is_fully_blocked|default_allow_read|default_allow_write|network\\.rules|allow_read|allow_write" crates/capsem-core/src crates/capsem-core/tests crates/capsem-service/src crates/capsem-gateway/src -g '*.rs'` has no active domain-decision type/field hits outside retired setting ids/tests; `web_default_toggles_not_exposed_as_guest_authority` proves stale web toggles do not produce guest env authority; `batch_update_rejects_retired_web_decision_setting_ids`, `migrate_setting_ids_does_not_resurrect_retired_web_decision_keys`, `retired_web_decision_settings_are_not_resolved`, and Python `TestRetiredWebDecisionConfig::test_allow_block_fields_fail_closed` prove retired web decision settings fail closed or remain inert stale input.
- E2E/VM: route-only VM utility slice deferred real VM execution to T7; `uv run python -m pytest --collect-only tests -q` proves all VM suites import with the new route contract.
- Telemetry/session DB: pending.
- Frontend: `pnpm --dir frontend test src/lib/__tests__/api.test.ts src/lib/__tests__/mcp-store.test.ts`; `pnpm --dir frontend test src/lib/__tests__/api.test.ts`; `pnpm --dir frontend test src/lib/__tests__/api.test.ts src/lib/__tests__/settings-store.test.ts`; `pnpm --dir frontend test src/lib/__tests__/api.test.ts src/lib/__tests__/settings-store.test.ts src/lib/models/__tests__/settings-model.test.ts`; `pnpm --dir frontend test src/lib/__tests__/settings-store.test.ts src/lib/models/__tests__/settings-model.test.ts`; `pnpm --dir frontend check`; `api.test.ts` proves settings calls `GET /settings/info` and `PATCH /settings/edit`, reload calls `POST /profiles/default/reload`, plugin API calls profile-scoped plugin routes and uses `PATCH`, MCP API calls profile/server-scoped routes, VM lifecycle helpers call `/vms/create`, `/vms/list`, `/vms/{id}/info`, `/vms/{id}/status`, and `/vms/{id}/stop|pause|delete|resume|save|fork`, VM utility helpers call `/vms/{id}/exec|logs|inspect` plus `/vms/{id}/files/read|write|list|content`, and no settings lint/preset helpers remain; settings model tests prove no preset accessor remains.
- Performance/benchmarks: pending.
- Install/package: pending.
- Docs/changelog: `CHANGELOG.md` updated for the MCP policy API/UI/CLI burn, retired web decision settings burn, profile-scoped plugin API, profile/server-scoped MCP API, profile-owned enforcement authoring API, `/corp/edit` replacement for retired `/corp-config`, `/settings/info|edit` replacement for retired magic `/settings`, removal of retired `/settings/presets`, `/settings/lint`, and `/settings/validate-key`, removal of preset metadata from `/settings/info`, profile reload replacement for retired `/reload-config`, VM-scoped ledger route replacement for retired `/security|detections|enforcements/{id}` routes, and VM core/lifecycle/utility route normalization under `/vms`.
