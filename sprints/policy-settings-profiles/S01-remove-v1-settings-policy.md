# S01 - Remove V1 Settings/Policy

## Goal

Remove old settings/policy authority completely while keeping Capsem bootable on
the new skeleton path.

## Requirements

- No compatibility layer.
- No migration layer.
- No special diagnostics for v1 shapes.
- `config/defaults.json` is not runtime or UI authority.
- Standalone `[mcp]` config authority is removed.

## Initial Proof Targets

- Capsem service starts.
- Base profile can load.
- Basic session creation path has a profile to resolve.
- Debug report does not depend on v1 settings tree.

## Tasks

- [x] Remove service `/settings*` handlers that read/write
  `net::policy_config` trees, and replace with typed settings/profile APIs.
- [x] Remove service `/mcp*` handlers that read merged v1 settings, and replace
  with typed profile/connector surfaces.
- [x] Remove `capsem-process` runtime dependence on
  `net::policy_config::MergedPolicies` and old settings files for guest policy,
  MCP policy, and snapshot defaults.
- [x] Remove `capsem-service` runtime dependence on
  `load_merged_vm_settings()` for VM resource defaults and limits.
- [x] Remove `config/defaults.json` authority and old settings-tree UI contract
  from frontend stores/routes/components.
- [x] Add focused service/process/frontend tests proving the v1 tree is not read
  for settings/profile/MCP runtime paths.
- [x] Add boot/session smoke verification proving a selected profile path works
  without v1 settings/policy fallback.

## Removal Map

- `crates/capsem-core/src/net/policy_config/registry.rs` owns
  `DEFAULTS_JSON`, `setting_definitions`, and the old UI/runtime metadata.
- `crates/capsem-core/src/net/policy_config/tree.rs`, `loader.rs`, and
  `resolver.rs` assemble old settings trees and batch update JSON.
- `crates/capsem-core/src/net/policy_config/builder.rs` builds legacy
  domain/http/MCP policy from settings and `MergedPolicies`.
- `crates/capsem-core/src/net/policy_config/types.rs` still accepts old
  settings shapes and standalone `[mcp]`.
- `crates/capsem-service/src/main.rs` exposes old `/settings` and `/mcp`
  routes that must move to the new service/profile APIs.
- `crates/capsem-process/src/main.rs` and `crates/capsem-process/src/ipc.rs`
  read `MergedPolicies` and need VM-effective profile assembly instead.
- `frontend/src/lib/api.ts`, settings stores, `McpSection.svelte`, and
  `PolicyRulesSection.svelte` depend on old route and tree shapes.
- Existing frontend/settings tests exercise the old UI contract and will need
  replacement, not compatibility.

## Implementation Notes

- A compile-tested typed replacement foundation now exists in
  `capsem-core::settings_profiles`.
- S03/S06 now provide typed service settings load plus VM-effective settings
  attachment to session directories.
- Next S01 step is to move live service/process/frontend callers off
  `net::policy_config` and then delete the old registry/tree surfaces.

## Kickoff Sequence (Ready To Execute)

1. Service cutover:
   remove `/settings*` and `/mcp*` v1 handlers and route those flows to typed
   settings/profile surfaces.
2. Process/runtime cutover:
   remove `MergedPolicies` and `load_merged_vm_settings()` runtime dependencies
   in service/process session launch paths.
3. Frontend cutover:
   move stores/components off `defaults.json` + v1 tree route contracts to typed
   settings/profile contracts.
4. Deletion pass:
   delete now-unreferenced `policy_config` registry/tree/builder surfaces and
   old v1 settings route code.
5. Focused verification:
   run service/process/frontend tests for touched paths, then run profile-backed
   boot/session smoke checks proving no v1 fallback remains.

## Immediate Verification Targets

- No runtime reads of `config/defaults.json` for settings/profile/MCP flows.
- No service/process runtime dependency on `MergedPolicies` for profile-backed
  session operations.
- Profile-backed session launch remains functional and debuggable.

## Coverage Ledger

- Unit/contract: deletion boundary and new skeleton loaders.
- Functional: service startup and base profile load.
- Adversarial: old config shapes are not recognized.
- E2E/VM: basic profile-backed session smoke.
- Telemetry: debug report no longer reads v1 tree.
- Performance: not primary.

## Execution Update (2026-05-14)

- Service runtime VM defaults now resolve from typed profile settings via
  `settings_profiles::resolve_effective_vm_settings` instead of
  `net::policy_config::load_merged_vm_settings`.
- `capsem-service` provisioning paths (`provision_sandbox`, `handle_provision`,
  `handle_run`) now source default RAM/CPU from the default profile VM section.
- `/mcp/servers` and `/mcp/policy` no longer read v1
  `net::policy_config::load_settings_files()`; they now resolve from typed
  effective profile settings plus MCP runtime tool-cache data.
- `capsem-process` runtime initialization and live reload paths no longer read
  `net::policy_config::load_settings_files()` or
  `net::policy_config::MergedPolicies`.
- `capsem-process` now loads policy/runtime state from session-attached
  `vm-effective-settings.toml` and converts it into runtime `NetworkPolicy`,
  `DomainPolicy`, `McpPolicy`, and Policy V2 rule maps.
- `ServiceToProcess::ReloadConfig` rebuilds from the vm-effective attachment
  path (not global v1 files). The old `McpRefreshTools` management IPC has
  since been deleted by S07's connector replacement.
- `/settings`, `/settings/presets`, `/settings/presets/{id}`, and
  `/settings/lint` now operate on typed `settings_profiles` service/profile
  state and no longer call v1 settings tree loader/update/preset/lint paths.
- `/settings` no longer emits legacy compatibility keys
  (`tree`, `issues`, `presets`, `policy`); the response now exposes strict
  typed fields: `settings_profiles`, `profile_presets`, and `effective_rules`.
- `/setup/corp-config` and CLI setup corp provisioning now accept only canonical
  profile TOML; legacy `[settings]` corp config shape is rejected with parse
  errors (fail closed, no compatibility shim).
- Snapshot scheduler limits in process now use explicit runtime defaults
  (10 auto snapshots, 12 manual snapshots, 300s interval) instead of
  v1 settings-tree resolved values.
- Frontend settings API/model no longer assume `/settings` returns legacy
  tree-shaped fields. Responses are normalized from strict
  `settings_profiles_v2` payloads, policy reads source from `effective_rules`,
  and tests now cover the adapter behavior.
- Verified with focused tests:
  - `cargo test -p capsem-process`
  - `cargo test -p capsem-service --no-run`
  - `cargo test -p capsem-service handle_`
  - `cargo test -p capsem-service mcp_`
  - `cargo test -p capsem setup:: -- --nocapture`
  - `cargo test -p capsem-service provision_`
  - `cargo test -p capsem-service run_request_`
  - `cargo test -p capsem-service ensure_vm_effective_settings_`
  - `cargo test -p capsem-core --no-run`
  - `cargo test -p capsem-process --no-run`
  - `uv run python -m pytest tests/capsem-service/test_svc_settings.py tests/capsem-service/test_svc_setup.py -v --tb=short`
    (18 passed)
  - `pnpm -C frontend test -- src/lib/__tests__/api.test.ts src/lib/models/__tests__/settings-model.test.ts`
    (19 files, 388 tests passed)
  - `pnpm -C frontend check` (Astro + svelte-check, 0 errors)
