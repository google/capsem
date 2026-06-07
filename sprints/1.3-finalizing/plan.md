# 1.3 Finalizing Sprint

## Purpose

Close the 1.3 branch cleanly without reintroducing old policy paths or hiding
unfinished security architecture behind UI/compatibility paint.

## Absolute Profile Contract

Capsem operates on independent profiles. A VM executes a profile.

This is the contract we promised and the code/docs/skills must reflect it:

- **Profile owns VM behavior.**
  - assets
  - VM/runtime config
  - security rules and enforcement defaults
  - detection rules
  - MCP servers/tools/config
  - skills
  - provider/model configuration
  - anything else that changes what a VM can do or what is observed/enforced
- **Settings are UI/application preferences.**
  - appearance
  - notifications
  - local UI behavior
  - other user-interface preferences that do not define VM behavior
- **Corp owns constraints and reporting.**
  - profile fields/rules the user cannot change
  - required reporting endpoints
  - detection/export integrations
  - enforcement constraints
  - any corporate lock/default that shapes profile behavior
- **Service owns only service-global state.**
  - daemon status
  - install/assets availability
  - service health
  - global process/runtime information that is genuinely one-per-service

Therefore, endpoints and config must be profile-addressed unless they are truly
service-global. Global enforcement/plugin/MCP endpoints are suspect by default.
The final architecture should be profile-first, e.g.
`/profiles/{profile_id}/enforcement/...`,
`/profiles/{profile_id}/detection/...`,
`/profiles/{profile_id}/plugins/...`, and
`/profiles/{profile_id}/mcp/...`.

## Required End Posture

The 1.3 cleanup is not done until the codebase matches this endpoint and
ownership posture:

- `api-contract.md` is the target API contract for this sprint.
- Endpoint path words are disciplined:
  - `info` means configuration/metadata.
  - `status` means runtime state, counters, readiness, or progress.
  - `list` means collection.
  - `latest` means DB-backed ledger rows.
  - `edit` means configuration mutation.
  - `reload` means re-read/apply owned config files.
- Profile authoring is profile-addressed. Anything that changes VM behavior
  belongs under `/profiles/{profile_id}/...`.
- Settings are UI/application preferences only. Settings must not own assets,
  VM config, enforcement, detection, MCP, skills, plugins, or credentials.
- Corp owns constraints, locks, and reporting endpoints over profiles.
- Service-global endpoints are runtime/reporting only:
  - daemon health/status,
  - service asset cache status,
  - VM runtime state,
  - DB-backed latest/status ledger views.
- A VM has an immutable assigned profile id. Changing profile means creating or
  forking a VM, not editing the existing VM.
- VM lifecycle must expose status plus explicit lifecycle verbs:
  `start`, `resume`, `pause`, `stop`, `restart`, `save`, `fork`, and
  `reload-profile` where supported.
- Per-VM mutable configuration uses `/vms/{vm_id}/edit`; it cannot change the
  VM's assigned profile.
- MCP tools, resources, and prompts are per server. There is no global MCP tool
  list.
- Plugin docs live on the docs site under `/plugins/...`; there is no plugin
  `man` endpoint.
- Provider is not a 1.3 profile API object. Credential brokerage plus rules own
  provider-like behavior.
- Enforcement/detection source files are represented through
  `/profiles/{profile_id}/enforcement/info`,
  `/profiles/{profile_id}/detection/info`, and their `reload` endpoints, not a
  generic `rule-files` API.
- HTTP and UDS must expose the same route, DTO, and error contract.

## Security Ownership Contract

Do not let endpoint cleanup blur the earlier security decisions. This is also
part of the 1.3 end posture:

- **Single decision rail.** All allow/ask/block/rewrite/preprocess/postprocess
  decisions are rules over typed security events and are evaluated by the
  security/CEL rule rail.
- **No MCP policy engine.** MCP can have server/tool/resource/prompt config and
  runtime discovery mechanics, but it cannot own an allow/ask/block decision
  provider. MCP decisions are profile rules over MCP security event fields.
- **No network policy decision engine.** The network engine owns parsing,
  capture, routing mechanics, DNS/proxy mechanics, ports, caching, connection
  reuse, body limits, decompression, and provider metadata. It does not own
  security decisions. HTTP/DNS/domain allow/block/ask lives in rules.
- **Network routing is mechanics, not policy.** We are not adding a separate
  `NetworkRouting` abstraction. Network mechanics stay inside the network
  engine; security decisions stay outside on the rule rail.
- **Default rules are real rules.** Built-in defaults compile into the same
  `SecurityRuleSet`; they are not a second engine and not a fallback shortcut.
- **Default priority is last.** `priority = "default"` is the only catch-all
  sentinel beyond numeric priorities. Specific corp/profile/user rules must
  evaluate before defaults.
- **Default rules are visible.** Defaults must be represented in profile rule
  lists with names, reasons, groups, priorities, and actions from the backend
  contract so the UI can show and mutate them without inventing copy.
- **Plugin effects are explicit event effects.** Plugins may mutate a security
  event, append detection events, and strengthen decisions through the plugin
  contract; block remains absolute. Plugins are not a second hidden policy
  system.
- **Runtime ledger is truth.** Detection/enforcement/latest/status endpoints
  report stored ledger facts and effects, not recomputed active policy state.
- **Security event abstraction is first-class.** HTTP, DNS, MCP, model, file,
  process, credential, and snapshot events must be represented as typed security
  events before rules/plugins operate on them.

## UI Reflection Contract

The UI is a view/editor over backend contract truth. It must not become a second
configuration model.

- The UI reads profile/corp/settings/runtime truth from the approved endpoints.
- The UI writes through approved endpoints only.
- The UI does not rename backend-owned objects:
  - profile names,
  - rule names,
  - rule reasons,
  - rule actions,
  - detection levels,
  - plugin names/descriptions,
  - MCP server/tool/resource/prompt names,
  - skill names/descriptions,
  - credential ids/hashes,
  - asset names/status.
- The UI does not invent explanatory text for backend-owned config. Backend
  `name`, `reason`, `description`, `status`, `source`, `group`, and validation
  messages are the source of truth.
- The UI may add presentation-only structure:
  - grouping,
  - sorting,
  - filtering,
  - tabs,
  - labels for UI-only controls,
  - button text/icons,
  - empty/loading/error shell states.
- UI grouping must come from backend fields when the group has config meaning
  (`rule.group`, `rule.source`, plugin scope, MCP server id, profile id). The UI
  can choose layout, but it cannot create semantic categories that do not exist
  in the contract.
- UI settings are UI/app preferences only. A frontend settings store must not
  carry VM behavior, security rules, MCP policy, plugin config, credentials, or
  assets.
- Frontend tests should assert rendered security/profile text comes from API
  fixtures, not hard-coded UI copy.

The current code and several docs/skills confuse `settings`, `profiles`, and
`corp`. Burning that ambiguity is a release blocker.

This sprint is a release finalization board. It must separate:

- confirmed 1.3 release blockers,
- open design questions,
- partial work already in the worktree,
- tests/smoke checks needed before asking Linux to finish validation.

## Current Partial Worktree State

There is uncommitted partial work from the default-rule discussion:

- `crates/capsem-core/src/net/policy_config/security_rule_profile.rs`
  - Added `profiles.defaults` as a visible grouping for default rules.
  - Added `priority = "default"` syntax compiling to a sentinel after numeric user priorities.
  - Added plugin reachability validation with a `dummy_*` exception.
- `crates/capsem-core/src/net/policy_config/default_provider_rules.toml`
  - Added default allow rules for HTTP, DNS, MCP, model, file, process, credential, and snapshot.
  - Moved them toward `profiles.defaults.*`.
  - Added `[plugins.credential_broker]`.
- `crates/capsem-core/src/net/policy_config/provider_profile.rs`
  - Began enforcing that built-in profiles contain real plugins and visible default rules.
- `crates/capsem-core/src/net/policy_config/builder.rs`
  - Began merging built-in plugin defaults into runtime plugin config.
- `crates/capsem-service/src/main.rs`
  - Began adding `/enforcements/list`.
- `crates/capsem-gateway/src/main.rs`
  - Began forwarding `/enforcements/list`.
- `frontend/src/lib/api.ts`
  - Began adding enforcement-list rule types/API.
- `frontend/src/lib/components/settings/PolicySection.svelte`
  - New partial UI for grouped policy rules.
- `frontend/src/lib/components/shell/SettingsPage.svelte`
  - Began wiring the Policy tab to `PolicySection`.
- `sprints/security-default-rule-rail/`
  - Scratch sprint created during the interrupted slice.

Do not commit this partial work until the design questions below are resolved.

## Design Questions To Resolve Before More Code

1. What is the concrete profile schema?
   - Current code has a `profiles` namespace/group but not a clear independent profile object.
   - Required direction: profile is the unit a VM executes.
   - Avoid fake profile fields or profile-less APIs pretending to be the final shape.

2. Are `profiles.defaults.*` the correct visible location for default rules inside a profile?
   - Current leaning: yes.
   - They are UX grouping only; they compile into the same `SecurityRuleSet`.

3. Should default rule compiled IDs be `profiles.rules.<id>` or `profiles.defaults.<id>`?
   - The UI needs defaults grouped.
   - Runtime override semantics need discipline. If a user tweaks a default, do we replace the built-in default or add a more specific user rule?

4. What should profile-addressed enforcement/detection list endpoints return?
   - It should not be a special defaults endpoint.
   - It should list normal profile enforcement rules and include enough fields to group defaults.
   - It should reflect contract fields (`rule.name`, `rule.reason`, `rule.action`, `priority`) without invented UI text.
   - Avoid global `/enforcements/list` as a final shape. Runtime ledger views are `/enforcement/latest|status`; authoring is `/profiles/{profile_id}/enforcement/rules/list`.

5. How should default plugins be enforced per profile?
   - If a real plugin exists in profile config, it should be reachable from at least one rule.
   - `dummy_*` debug plugins are exempt.
   - Separate invariant: shipped default profile must contain required real plugin config such as `credential_broker`.

6. How should raw enforcement/Sigma file preview/edit work per profile?
   - UI must not invent file paths or content.
   - Need backend contract exposing enforcement and detection file references/content before adding raw editors.
   - Future UI can use an existing editor if available, but only once backend exposes the truth.

7. Which current "settings" are actually profile-owned?
   - Anything affecting VM behavior or security belongs to profile, not UI settings.
   - UI settings remain app/UI preferences only.

## Required 1.3 Cleanup Tasks

### Security Rule Defaults

- [ ] Decide final compiled ID semantics for `profiles.defaults`.
- [ ] Keep default rules visible in config, grouped as defaults.
- [ ] Keep `priority = "default"` as UX sugar for the last catch-all tier.
- [ ] Ensure numeric priorities remain bounded to `[-1000, 1000]`.
- [ ] Ensure `priority = "default"` is the only max+1 sentinel.
- [ ] Ensure default rule descriptions/reasons name user-facing objects:
  - HTTP requests
  - DNS queries
  - MCP tool/server activity
  - model calls
  - file activity
  - process activity
  - brokered credential references
  - snapshot actions
- [ ] Add tests proving specific corp/user rules win before default catch-alls.
- [ ] Add tests proving default catch-alls cover non-matching events.
- [ ] Add tests proving mutating a default rule changes evaluation behavior.

### Plugin Contract

- [ ] Decide exact required built-in plugin set for 1.3.
- [ ] Enforce shipped profile contains required plugin configs.
- [ ] Enforce real configured plugins are referenced by rules.
- [ ] Keep `dummy_*` plugin exception for endpoint/debug tests.
- [ ] Confirm plugin list UI reflects backend plugin `id`, mode, detection level, and backend description only.
- [ ] Do not invent plugin names/descriptions in UI.

### Enforcement And Detection API

- [ ] Replace global enforcement/detection API assumptions with profile-addressed API shape.
- [ ] Finalize `/profiles/{profile_id}/enforcement/rules/list` response shape.
- [ ] Add equivalent `/profiles/{profile_id}/detection/rules/list` if detection rules are distinct in the API.
- [ ] Keep latest/info endpoints backed by the ledger tables, not rebuilt from active rules.
- [ ] Make sure enforcement list groups defaults but treats them as normal rules.
- [ ] Decide whether rule mutation should support default-group writes directly or only normal user overrides.
- [ ] Do not add `/enforcements/defaults`.
- [ ] Do not add fake profile fields. Implement real profile addressing or keep the work out of 1.3.

### Profile/Settings/Corp Architecture

- [ ] Define the canonical profile schema.
- [ ] Move VM behavior config out of the UI settings mental model and into profile.
- [ ] Keep UI settings limited to app/UI preferences.
- [ ] Define corp overlay/lock semantics over profiles.
- [ ] Define how a VM selects/executes a profile.
- [ ] Audit config code for violations of the profile contract.
- [ ] Audit service/gateway routes for global endpoints that should be profile-addressed.
- [ ] Audit frontend settings pages for profile-owned controls rendered as UI settings.
- [ ] Update architecture docs.
- [ ] Update project skills that describe config/settings/profile behavior.

### UI Policy Page

- [ ] Replace partial `PolicySection.svelte` with the agreed contract shape.
- [ ] Group defaults in the Policy page.
- [ ] Render rule names from `rule.name`.
- [ ] Render rule descriptions from `rule.reason`.
- [ ] Render action from `rule.action`.
- [ ] Allow tweaking default actions only if backend semantics are settled.
- [ ] Show plugin controls in the policy/settings area using backend plugin metadata.
- [ ] Add raw enforcement/Sigma file preview/edit only after backend exposes file references/content.
- [ ] Add frontend tests for grouping and contract text.

### Old Policy Burn Pass

- [ ] Re-check there is no live `NetworkPolicy::evaluate` enforcement path.
- [ ] Re-check MCP policy permission fields are not live enforcement.
- [ ] Decide what remains as network-engine mechanics:
  - HTTP upstream ports
  - DNS redirects
  - DNS cache
  - body capture limits
- [ ] Remove or rename old policy wording where it misrepresents mechanics as policy.
- [ ] Keep all allow/ask/block decisions on the CEL/security-rule rail.

### Release Verification

- [ ] Run focused Rust rule/security tests.
- [ ] Run service tests around enforcement/plugin endpoints.
- [ ] Run frontend typecheck/tests for the Policy page.
- [ ] Run smoke install/start check.
- [ ] Confirm assets status works in UI.
- [ ] Confirm EROFS LZ4HC default and kernel state in docs/changelog.
- [ ] Confirm Linux-only KVM/EROFS/DAX items are documented for Linux team validation.
- [ ] Confirm changelog says only what is implemented.
- [ ] Confirm docs describe the current rule syntax and default-rule grouping.

## Out Of Scope Unless We Explicitly Pull It In

- Any implementation that leaves profile semantics ambiguous.
- Raw rule-file Monaco editor without backend file contracts.
- YARA.
- Any resurrection of old policy-v2/domain/MCP decision providers.
- New network routing abstraction.

## Testing Ledger

- Unit/contract: pending.
- Functional API: pending.
- Frontend: pending.
- E2E/VM: pending.
- Session DB/ledger: pending.
- Linux validation: pending, expected to be completed by Linux team for KVM-specific paths.
