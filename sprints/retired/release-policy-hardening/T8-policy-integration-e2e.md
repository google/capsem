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

## Decision Record

- 2026-05-10: configured external policy hook dispatch is deferred for
  `1.1.1778445002`.
- Shipping scope: non-hook Policy V2 enforcement, Policy Hook Spec0 document,
  hardened hook client/spec/audit infrastructure, and service `/policy-hook/spec`.
- Deferred scope: hook endpoint settings/defaults, endpoint persistence,
  service/process hook endpoint propagation, production calls to
  `PolicyHookClient::decide`, `hook.decision` UI/rule editing, and black-box
  hook dispatch E2E.
- Release guardrails: frontend editing/import rejects `policy.hook.*`, backend
  `/settings` rejects new `policy.hook.*` writes, and docs/release notes call
  configured external hook dispatch infrastructure-only.

## Swarm Transfer Tracker

| Source | Priority | Owner task | Required transfer point | Required proof |
|---|---:|---|---|---|
| FD01 ui-policy-settings | P0 | T8.1, T8.3 | UI hook controls cannot remain exposed until production hook dispatch/config scope is decided. | T8.1 records ship/defer decision; T2 tests and T4/T9 wording match it. |
| FD01 ui-policy-settings | P0 | T8.6 | Callback/runtime support matrix must cover `dns.response`, `hook.decision`, and `rewrite`. | Matrix names runtime code path, telemetry proof, and frontend validation owner for each visible callback. |
| FD01 ui-policy-settings | P0 | T8.4 | Settings save/apply reload failure must be proved across UI/store/service/process. | Running-session apply test proves live reload or saved-not-applied failure path. |
| FD01 ui-policy-settings | P1 | T8.6 | Runtime/image truth needs ship/defer matrix for asset health, image/fork UI, create defaults, service status, and gateway status. | T8.6 matrix plus T2.8 UI proof covers every shipped or hidden surface. |
| FD04 core-policy-assets | P0 | T8.1, T8.2, T8.3 | Core accepts hook config but no production caller invokes `PolicyHookClient::decide`. | Hook ships with black-box dispatch proof and `policy_hook_events`, or controls/docs are hidden/deferred. |
| FD04 core-policy-assets | P0 | T8.4, T8.5 | MCP notification bypass must be closed in integration path as well as unit tests. | E2E/integration proof shows no notification bypass and telemetry/audit row exists. |
| FD05 service-process | P1 | T8.4 | Settings apply/reload and builtin MCP domain policy are startup-only or masked by success responses. | Running VM proof shows updated policy/domain state or explicit failed-session reporting. |
| FD05 service-process | P1 | T8.1, T8.2, T8.3 | Hook dispatch is not integrated through service/process. | Same hook ship/defer decision with production-path proof or release-honest hiding. |
| FD07 mcp-policy-boundary | P1 | T8.4, T8.5 | Builtin HTTP redirects, refresh behavior, policy denial telemetry, and trace propagation affect the production policy story. | E2E proof covers updated domain policy, redirect denial, timeline/session telemetry, and trace continuity where shipped. |
| FD08 telemetry-session | P1 | T8.5 | Timeline/triage and session tooling must expose the policy/hook evidence used for release claims. | T6 proof is linked from T8 and inspected during Gate B. |
| FD11 verification-architecture | P1 | T8.1 | T8 is a decision fork, not one implementable sprint. | T8.1 closes before T2/T3/T4/T6/T9 release-facing decisions finalize. |
| FD11 verification-architecture | P1 | T8.6 | Frontend runtime/image truth must be owned explicitly. | T8.6 stays open until T2.8 is proved or surfaces are hidden. |
| FD14 swarm-transfer-closeout | P1 | T8.6 | Runtime/image truth placeholder references must become executable owner tasks. | No unresolved placeholder owner text remains outside finding docs. |

## Task List

### T8.1 Decide Shipping Scope

- [x] Decide whether configured external hook dispatch ships in `1.1.1778445002`.
- [x] If it ships, define endpoint settings/defaults, persistence, reload, and
  dispatch boundaries. Decision: it does not ship in `1.1.1778445002`; these remain
  post-1.1 work.
- [x] If it does not ship, hide/disable `hook` rule type and `hook.decision`
  callback in T2 and document infrastructure-only status in T4.
- [x] Record the decision in `MASTER.md`, `tracker.md`, T2, T3, and T4.

### T8.2 If Hook Dispatch Ships

- [x] Deferred post-1.1: add hook endpoint settings/defaults and persist them
  in `SettingsFile`.
- [x] Deferred post-1.1: expose settings through service settings read/write.
- [x] Deferred post-1.1: propagate endpoint config into process runtime and
  reload.
- [x] Deferred post-1.1: call `PolicyHookClient::decide` from the intended
  callback boundaries.
- [x] Deferred post-1.1: define local policy plus hook decision precedence.
- [x] Deferred post-1.1: after every valid hook response, evaluate
  `PolicyCallback::HookDecision`
  rules against the hook-decision subject and enforce the documented
  precedence.
- [x] Deferred post-1.1 production path; existing hook infrastructure tests
  ensure hook timeout/body/schema failures fail closed and write
  `policy_hook_events`.
- [x] Deferred post-1.1: add black-box E2E proving a hook block prevents
  dispatch.
- [x] Deferred post-1.1: create `tests/capsem-e2e/test_policy_hook_runtime.py`
  with
  `test_settings_hook_block_prevents_dispatch_records_policy_hook_event` before
  the conditional hook-ships verification command is promoted to a final gate.
- [x] Deferred post-1.1: add a production-path test where a hook returns
  `allow` but a
  `policy.hook.*` rule blocks the hook decision and records telemetry.

### T8.3 If Hook Dispatch Does Not Ship

- [x] Hide/disable hook rule type and `hook.decision` callback in the frontend.
- [x] Keep hook OpenAPI/spec/client docs scoped as infrastructure.
- [x] Add tests ensuring non-shipping hook controls are not visible in Settings.
- [x] Reword release docs/changelog through T4.

### T8.4 Running Session Apply Semantics

- [x] Define the frontend reload-failure state: `persisted = true`,
  `applied = false`, `failed_session_count`, `failed_session_ids` when
  available, `message`, and retry action.
- [x] Settings UI must show a persistent saved-but-not-applied banner until
  retry succeeds, settings are changed again, or all affected sessions stop.
- [x] Add a store/component test for failed reload, retry success, and dismissal
  rules.
- [x] Update running-VM E2E path to save Policy V2 through `/settings`, call
  `/reload-config`, and prove the existing MCP connection sees the new rule.
  Focused VM proof passed during T10 on 2026-05-10.
- [x] On reload failure, prove the UI reports saved-but-not-applied to running
  sessions.
- [x] Refresh builtin MCP env or move builtin HTTP enforcement to live shared
  policy; T8 E2E now warms builtin HTTP before settings reload and checks the
  refreshed domain policy.
- [x] Ensure `McpRefreshTools` uses builtin-aware server list construction.

### T8.5 Telemetry and Debug Surfaces

- [x] Add one telemetry assertion that a Policy V2 decision or hook fallback is
  visible through session DB and timeline tooling; T8 adds a real
  `/timeline/{id}?layers=mcp` assertion to the running-session reload E2E.
- [x] Add DNS/hook/audit/snapshot timeline layers through T6 if needed for the
  proof.
- [x] Expose MCP policy fields in frontend session/tool SQL if the UI claims
  policy auditability.
- [x] Normalize MCP policy mode/action naming with T3.

### T8.6 Runtime Support Matrix

- [x] Record which runtime-facing UI surfaces ship in `1.1.1778445002`: Policy V2,
  hook controls, image/fork selection, asset health, create defaults, service
  status, and gateway status.
- [x] For each shipped surface, name the production route/config source,
  reload behavior, error state, telemetry/log proof, and UI test owner.
- [x] For each deferred surface, ensure T2 hides the control or labels it as
  non-shipping outside release UI.
- [x] Prove the create flow uses the same defaults and image/source semantics
  as the CLI/service path.
- [x] Add one E2E or integration proof that the frontend's runtime-ready state
  matches the service/gateway response for assets and session creation.

## Runtime Support Matrix

| Surface | 1.1 scope | Production source / route | Reload / error behavior | Proof owner |
|---|---|---|---|---|
| Policy V2 MCP/HTTP/DNS/model rules | Ships, excluding configured external hook dispatch | `/settings` `policy.{mcp,http,dns,model}.*`, `SettingsFile.policy`, process reload IPC | `/reload-config` applies to running sessions or returns structured failed-session state | T2 model/component tests; T8 framed MCP E2E path; T10 focused VM proof |
| Hook controls and `hook.decision` | Deferred / infrastructure-only | `/policy-hook/spec`, `config/policy-hook-openapi.json`, hook client unit paths | No endpoint config or production dispatch; new `policy.hook.*` writes rejected | T8.1/T8.3; T9 final release wording |
| Image/fork selection | Deferred as a user-selectable UI surface | Service provision path keeps `from` internal/CLI-facing | No Settings UI selector for this release | T2.8 runtime truth tests; T10 visual proof |
| Asset health | Ships | `/status` gateway/service asset readiness | Unknown or missing assets disable session creation and explain missing items | `session-runtime-truth.test.ts`; T10 Gate A/B |
| Create defaults | Ships | Quick create omits CPU/RAM so service defaults apply; override mode sends explicit values | Creation disabled when assets are not ready | `session-runtime-truth.test.ts`; T10 Gate A/B |
| Service and gateway status | Ships | Gateway `/status` and service status fields | UI reports unavailable/unknown instead of assuming readiness | T2.8 runtime truth tests; T10 visual proof |

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

- [x] Scope decision recorded in tracker and docs.
- [x] Non-hook shipped Policy V2 proof:
  `uv run pytest tests/capsem-e2e/test_policy_v2_http_dns_mitm.py::test_guest_http_policy_v2_block_and_header_strip_records_session_db -q`
- [x] Running-session reload proof:
  `uv run pytest tests/capsem-e2e/test_framed_mcp_mitm.py::test_framed_guest_mcp_policy_reload_blocks_existing_connection -q`
- [ ] If hook ships: black-box MCP/HTTP/DNS/model test proving hook block
  prevents dispatch and writes `policy_hook_events`.
- [ ] If hook dispatch ships, add and run:
  `uv run pytest tests/capsem-e2e/test_policy_hook_runtime.py::test_settings_hook_block_prevents_dispatch_records_policy_hook_event -q`
- [x] If hook does not ship: frontend test proving hook controls are hidden or
  disabled.
- [x] Running-session reload store/component test proving policy applies or
  saved-not-applied
  error is surfaced.
- [x] Timeline/session assertion for policy decision or fallback added to the
  focused E2E path and passed during T10.
- [x] Runtime support matrix recorded in tracker and T2/T4/T9.
- [x] Frontend runtime/image truth proof from T2.8, or deferred surfaces hidden.

Verification note: `uv run pytest
tests/capsem-e2e/test_framed_mcp_mitm.py::test_framed_guest_mcp_policy_reload_blocks_existing_connection
-q` initially could not run in the sandbox because `uv` could not access
`/Users/elie/.cache/uv`. The T10 escalated rerun passed on 2026-05-10 with 1
test passing after tightening the telemetry assertion to require redaction of
denied arguments.

Policy confidence rerun: on 2026-05-10,
`env UV_CACHE_DIR=target/uv-cache uv run pytest
tests/capsem-e2e/test_framed_mcp_mitm.py::test_framed_guest_mcp_policy_reload_blocks_existing_connection
-q` passed (1 test), and `env UV_CACHE_DIR=target/uv-cache uv run pytest
tests/capsem-e2e/test_policy_v2_http_dns_mitm.py::test_guest_http_policy_v2_block_and_header_strip_records_session_db
-q` passed (1 test). This proves shipped non-hook Policy V2 enforcement for
the MCP reload path and HTTP/DNS MITM/session-db path. Configured external hook
dispatch remains explicitly deferred post-1.1.

## Exit Criteria

- [x] No UI or docs surface implies configured hook dispatch unless it is wired
  and tested.
- [x] Running sessions cannot silently keep stale policy after a successful
  settings save; reload failures return structured failed-session state and the
  Settings UI shows saved-but-not-applied until retry, settings change, or all
  affected sessions stop.
- [ ] At least one production-path policy decision has E2E runtime and
  telemetry proof, or the release explicitly scopes Policy V2 to non-hook
  enforcement.
- [x] Runtime-facing UI surfaces match the production support matrix.
