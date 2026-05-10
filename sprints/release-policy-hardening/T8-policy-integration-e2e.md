# T8: Policy Integration E2E

## Objective

Resolve the release truth for Policy V2 and hook integration from UI settings
through service config, process reload, runtime enforcement, and telemetry. The
release must either ship a proved production path or hide/document any
non-shipping surfaces.

## Owned Files

- `config/defaults.toml`
- `config/defaults.json`
- `frontend/src/lib/models/settings-model.ts`
- `frontend/src/lib/components/settings/PolicyRulesSection.svelte`
- `frontend/src/lib/stores/settings.svelte.ts`
- `crates/capsem-core/src/net/policy_config/types.rs`
- `crates/capsem-core/src/net/policy_hook.rs`
- `crates/capsem-core/src/net/mitm_proxy/mcp_frame.rs`
- `crates/capsem-service/src/main.rs`
- `crates/capsem-process/src/ipc.rs`
- `crates/capsem-process/src/main.rs`
- `crates/capsem-mcp-builtin/src/main.rs`
- `crates/capsem-logger/src/schema.rs`
- `frontend/src/lib/sql.ts`
- relevant E2E tests under `tests/capsem-*`

## Findings

- [P0] Policy hook is exposed in UI/config as `hook` and `hook.decision`, and
  core accepts `PolicyRuleType::Hook`, but `SettingsFile` has no hook endpoint
  config and no production path loads endpoints or calls `PolicyHookClient`.
- [P1] Frontend runtime/image truth needs a release support matrix so asset
  health, image/fork UI, create defaults, service status, and gateway status do
  not ship with unsupported or stale assumptions.
- [P1] Settings save can report success while running VMs keep stale policy if
  `/reload-config` fails and the frontend swallows the error.
- [P1] Built-in MCP policy/domain state is loaded from startup env and is not
  refreshed end-to-end after `ReloadConfig`.
- [P2] MCP Policy V2 telemetry can say `audit_only` for enforced blocks, maps
  block to `deny`, and drops non-blocking matches before logging.
- [P2] DNS policy blocks, hook failures, and policy fields are missing from
  primary triage/timeline/UI paths.

## Task List

### T8.1 Decide Shipping Scope

- [ ] Decide whether configured external hook dispatch ships in `1.1.xxx`.
- [ ] If it ships, define endpoint settings/defaults, persistence, reload, and
  dispatch boundaries.
- [ ] If it does not ship, hide/disable `hook` rule type and `hook.decision`
  callback in T2 and document infrastructure-only status in T4.
- [ ] Record the decision in `MASTER.md`, `tracker.md`, T2, T3, and T4.

### T8.2 If Hook Dispatch Ships

- [ ] Add hook endpoint settings/defaults and persist them in `SettingsFile`.
- [ ] Expose settings through service settings read/write.
- [ ] Propagate endpoint config into process runtime and reload.
- [ ] Call `PolicyHookClient::decide` from the intended callback boundaries.
- [ ] Define local policy plus hook decision precedence.
- [ ] After every valid hook response, evaluate `PolicyCallback::HookDecision`
  rules against the hook-decision subject and enforce the documented
  precedence.
- [ ] Ensure hook timeout/body/schema failures fail closed and write
  `policy_hook_events`.
- [ ] Add black-box E2E proving a hook block prevents dispatch.
- [ ] Add a production-path test where a hook returns `allow` but a
  `policy.hook.*` rule blocks the hook decision and records telemetry.

### T8.3 If Hook Dispatch Does Not Ship

- [ ] Hide/disable hook rule type and `hook.decision` callback in the frontend.
- [ ] Keep hook OpenAPI/spec/client docs scoped as infrastructure.
- [ ] Add tests ensuring non-shipping hook controls are not visible in Settings.
- [ ] Reword release docs/changelog through T4.

### T8.4 Running Session Apply Semantics

- [ ] Define the frontend reload-failure state: `persisted = true`,
  `applied = false`, `failed_session_count`, `failed_session_ids` when
  available, `message`, and retry action.
- [ ] Settings UI must show a persistent saved-but-not-applied banner until
  retry succeeds, settings are changed again, or all affected sessions stop.
- [ ] Add a store/component test for failed reload, retry success, and dismissal
  rules.
- [ ] Save settings while a VM is running and prove `ReloadConfig` updates live
  policy/domain/MCP state.
- [ ] On reload failure, prove the UI reports saved-but-not-applied to running
  sessions.
- [ ] Refresh builtin MCP env or move builtin HTTP enforcement to live shared
  policy.
- [ ] Ensure `McpRefreshTools` uses builtin-aware server list construction.

### T8.5 Telemetry and Debug Surfaces

- [ ] Add one telemetry assertion that a Policy V2 decision or hook fallback is
  visible through session DB and timeline tooling.
- [ ] Add DNS/hook/audit/snapshot timeline layers through T6 if needed for the
  proof.
- [ ] Expose MCP policy fields in frontend session/tool SQL if the UI claims
  policy auditability.
- [ ] Normalize MCP policy mode/action naming with T3.

### T8.6 Runtime Support Matrix

- [ ] Record which runtime-facing UI surfaces ship in `1.1.xxx`: Policy V2,
  hook controls, image/fork selection, asset health, create defaults, service
  status, and gateway status.
- [ ] For each shipped surface, name the production route/config source,
  reload behavior, error state, telemetry/log proof, and UI test owner.
- [ ] For each deferred surface, ensure T2 hides the control or labels it as
  non-shipping outside release UI.
- [ ] Prove the create flow uses the same defaults and image/source semantics
  as the CLI/service path.
- [ ] Add one E2E or integration proof that the frontend's runtime-ready state
  matches the service/gateway response for assets and session creation.

## Proof Matrix

| Category | Required proof |
|---|---|
| Unit/contract | settings/config parse the chosen hook scope correctly. |
| Functional | service save/reload applies policy to process runtime or reports failure. |
| Adversarial | unsupported hook controls are hidden/rejected, or malicious hook failures block safely. |
| E2E/VM | one production-path policy decision is proved from settings/config through runtime. |
| Telemetry | decision/fallback appears in session DB and timeline/tooling. |
| Runtime truth | frontend runtime/image/service readiness matches production support matrix. |
| Missing/deferred | if hook dispatch or image/fork UI is deferred, docs/UI explicitly say infrastructure only or hide it. |

## Verification

- [ ] Scope decision recorded in tracker and docs.
- [ ] Non-hook shipped Policy V2 proof:
  `uv run pytest tests/capsem-e2e/test_policy_v2_http_dns_mitm.py::test_guest_http_policy_v2_block_and_header_strip_records_session_db -q`
- [ ] Running-session reload proof:
  `uv run pytest tests/capsem-e2e/test_framed_mcp_mitm.py::test_framed_guest_mcp_policy_reload_blocks_existing_connection -q`
- [ ] If hook ships: black-box MCP/HTTP/DNS/model test proving hook block
  prevents dispatch and writes `policy_hook_events`.
- [ ] If hook dispatch ships, add and run:
  `uv run pytest tests/capsem-e2e/test_policy_hook_runtime.py::test_settings_hook_block_prevents_dispatch_records_policy_hook_event -q`
- [ ] If hook does not ship: frontend test proving hook controls are hidden or
  disabled.
- [ ] Running-session reload test proving policy applies or saved-not-applied
  error is surfaced.
- [ ] Timeline/session assertion for policy decision or fallback.
- [ ] Runtime support matrix recorded in tracker and T2/T4/T9.
- [ ] Frontend runtime/image truth proof from T2.8, or deferred surfaces hidden.

## Exit Criteria

- [ ] No UI or docs surface implies configured hook dispatch unless it is wired
  and tested.
- [ ] Running sessions cannot silently keep stale policy after a successful
  settings save.
- [ ] At least one production-path policy decision has E2E runtime and
  telemetry proof, or the release explicitly scopes Policy V2 to non-hook
  enforcement.
- [ ] Runtime-facing UI surfaces match the production support matrix.
