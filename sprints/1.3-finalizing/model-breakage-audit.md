# 1.3 Model Breakage Audit

Status: initial audit after approving the endpoint/profile posture.

## Target Model

- Profile owns VM behavior.
- Settings are UI/app preferences only.
- Corp owns constraints, locks, and reporting endpoints.
- Service-global endpoints are runtime/reporting only.
- VM assigned profile id is immutable.
- Single CEL/security-rule rail owns allow/ask/block decisions.
- Network engine owns parsing/capture/routing mechanics, not security
  decisions.
- MCP owns server/tool/resource/prompt config and discovery mechanics, not
  security decisions.
- Default rules are real visible rules in the same `SecurityRuleSet`, evaluated
  after specific corp/profile/user rules.
- Plugins can mutate events, append detections, and strengthen decisions through
  explicit event effects; they are not a hidden second policy engine.
- MCP tools/resources/prompts are per server.
- Provider is not a 1.3 profile API object; credentials plus rules own that
  behavior.

## P0 Breaks

### Service Routes Still Expose Old Global Authoring API

Evidence: `crates/capsem-service/src/main.rs:5531`.

Current service routes still expose:

- `/provision`, `/list`, `/info/{id}` instead of `/vms/create`,
  `/vms/list`, `/vms/{vm_id}/info`.
- `/suspend/{id}` instead of `/vms/{vm_id}/pause`.
- `/persist/{id}` instead of `/vms/{vm_id}/save`.
- `/fork/{id}` instead of `/vms/{vm_id}/fork`.
- `/resume/{name}` resumes by name, not immutable VM id.
- `/security/{id}/info`, `/detections/{id}/info`, and
  `/enforcements/{id}/info` use `info` for ledger counters; target is
  `status`.
- `/enforcements/list`, `/enforcements/evaluate`,
  `/enforcements/rules/{rule_id}`, `/enforcements/reload` are global authoring
  endpoints; target is `/profiles/{profile_id}/enforcement/...`.
- `/plugins`, `/plugins/global/{plugin_id}`, `/plugins/{id}` are global or
  VM-scoped plugin authoring endpoints; target is profile-scoped plugins.
- `/settings` owned behavior config behind a magic GET/POST route. The route is
  now split into `GET /settings/info` and `PATCH /settings/edit`; the remaining
  target is making the backing settings tree UI/app preferences only.
- `/corp-config` was a single mutation endpoint; `PUT /corp/edit` is now live
  and the retired route fails closed. Remaining target routes are `/corp/info`,
  `/corp/validate`, and `/corp/reload`.
- `/mcp/tools`, `/mcp/policy`, `/mcp/tools/refresh`, and tool approval/call
  endpoints are global MCP surfaces; target MCP tools/resources/prompts are
  under `/profiles/{profile_id}/mcp/servers/{server_id}/...`.

### Gateway Mirrors The Same Old Surface

Evidence: `crates/capsem-gateway/src/main.rs:218`.

Gateway proxy routes mirror the service's old route set. The gateway must be
updated in lock-step with service routes because HTTP and UDS must expose the
same contract.

### Config Builder Still Treats Settings As Behavior Owner

Evidence: `crates/capsem-core/src/net/policy_config/builder.rs:409`.

`MergedPolicies` is built from `SettingsFile` and still produces:

- `NetworkPolicy`
- `McpPolicy`
- `SecurityRuleSet`
- `plugins`
- `model_endpoints`
- `guest`
- `vm`

This breaks the target model in two ways:

- VM behavior is still settings-derived instead of profile-owned.
- `NetworkPolicy` and `McpPolicy` are still parallel decision objects beside
  `SecurityRuleSet`.

### MCP Policy Is Still A Decision Engine

Evidence:

- `crates/capsem-core/src/mcp/policy.rs:14`
- `crates/capsem-core/src/mcp/policy.rs:189`
- `crates/capsem-service/src/api.rs:315`

`McpUserConfig` still has `global_policy`, `default_tool_permission`, and
`tool_permissions`; `McpPolicy::evaluate()` still returns allow/warn/block.
That violates "MCP decisions are rules over security events."

### NetworkPolicy Still Encodes Domain Allow/Block Decisions

Evidence:

- `crates/capsem-core/src/net/policy_config/builder.rs:526`
- `crates/capsem-core/src/net/policy.rs:224`

`NetworkPolicy` still has domain read/write allow/block defaults and an
`evaluate()` function. Some network mechanics may remain, but allow/block
decisions must move to the CEL/security-rule rail.

## P1 Breaks

### Frontend API Uses Old VM Lifecycle

Evidence: `frontend/src/lib/api.ts:267`.

Current frontend functions call:

- `/provision`
- `/stop/{id}`
- `/suspend/{id}`
- `/resume/{name}`
- `/persist/{id}`
- `/fork/{id}`

Target functions should use `/vms/...` and expose `pause`, `resume`, `save`,
`fork`, and `status`. VM profile id must not be editable.

### Frontend Settings Store Owns VM/Security Behavior

Evidence:

- `frontend/src/lib/api.ts:621`
- `frontend/src/lib/stores/settings.svelte.ts:1`

The settings store loads and saves `/settings`, and tests/use sites stage
behavior fields like `vm.resources.cpu_count`, `security.web.allow_read`, and
AI provider settings. This contradicts settings-as-UI-only.

### Frontend MCP Store Assumes Global Tools And Policy

Evidence:

- `frontend/src/lib/api.ts:688`
- `frontend/src/lib/stores/mcp.svelte.ts:1`

The MCP store loads global servers, global tools, and global policy from
settings. Target model requires profile-scoped MCP servers, then tools/resources
/prompts under each server.

### Frontend Plugin API Is Global/VM-Scoped

Evidence: `frontend/src/lib/api.ts:650`.

`listPlugins(vmId?)` and `/plugins/global/{plugin_id}` encode old global/VM
plugin scopes. Target scope is profile/corp config.

### Enforcement API Is Global

Evidence: `frontend/src/lib/api.ts:670`.

Frontend calls `/enforcements/list`, `/enforcements/rules/{rule_id}`, and
`/enforcements/reload`. Target is profile-scoped enforcement.

## P2 Breaks / Docs Drift

### Old Settings Terminology Remains In Config Code

Evidence:

- `crates/capsem-core/src/net/policy_config/loader.rs`
- `crates/capsem-core/src/net/policy_config/types.rs`
- `crates/capsem-core/src/net/policy_config/tests.rs`

The loader still has `SettingsFile`, `[settings]`, `rule_files`, `mcp`, `ai`,
`plugins`, `profiles`, and `corp` in one file model. Some of this can be mapped
to the new profile/corp contract, but the current naming keeps the old mental
model alive.

## Recommended Cleanup Order

1. **Route contract slice**
   - Add/rename service and gateway routes to approved endpoint posture.
   - Keep HTTP and UDS identical.
   - Remove old global authoring routes once frontend/CLI callers move.

2. **Profile config object slice**
   - Define the profile-owned config DTO/schema.
   - Move behavior fields out of settings response.
   - Keep settings response UI-only.

3. **Security rail slice**
   - Remove `McpPolicy` decision use.
   - Reduce `NetworkPolicy` to mechanics only or rename/split mechanics out.
   - Ensure allow/ask/block decisions come from `SecurityRuleSet`.

4. **Frontend API/store slice**
   - Replace settings-owned behavior stores with profile-owned stores.
   - Replace MCP global tools/policy store with profile/server-scoped MCP store.
   - Replace global enforcement/plugin APIs with profile APIs.

5. **VM lifecycle slice**
   - Normalize frontend/service/gateway/CLI around `/vms/{vm_id}/...`.
   - Ensure profile id is immutable.
   - Add `pause`, `resume`, `save`, `fork`, and operation status surfaces.

6. **Docs/tests slice**
   - Update architecture/docs/skills to remove old settings-as-behavior model.
   - Add route conformance tests for approved endpoint vocabulary.
   - Add regression tests rejecting old global authoring endpoints.
