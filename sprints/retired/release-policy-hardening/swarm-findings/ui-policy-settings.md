# UI Policy Settings Findings

Status: completed; transferred to T7 FD01 and owner rows in T2/T8/T10.
T2 frontend implementation is complete; T8 runtime-scope and T10 visual/live
proof remain open.

Primary agent: Jason (`019e1263-534b-7702-864a-ca1f7b3a4f74`)
T2 execution audit agent: Boole (`019e12e6-3d40-70e1-b10e-3c9c4d09e6e1`)
T8 hook scope audit agent: Gibbs (`019e1342-9f35-7261-a62f-953938ceb395`)
T8 reload/telemetry audit agent: Mendel (`019e1342-9fe8-7b81-b5cb-39d3712ef196`)

## Scope

- Frontend Settings support for Policy V2.
- Staged pending state for add/edit/rename/delete/import/generated rules.
- Hook-control truthfulness if configured hook dispatch does not ship.
- Visual and component coverage.
- UI surfaces that show image/asset/runtime policy state.

## Findings

- [x] [P0] Hook policy remains exposed in Settings UI even though T8 has not
  resolved whether production hook dispatch/config ships. Either T8 must ship
  hook runtime or T2.6 must hide hook controls.
  - Paths: `frontend/src/lib/components/settings/PolicyRulesSection.svelte`,
    `frontend/src/lib/models/settings-model.ts`.
  - Proof: component tests now hide unsupported hook callbacks/decisions; T8
    scope audit chose defer for configured external hook dispatch. Frontend
    import/staging and backend settings writes reject new `policy.hook.*`
    entries for this release.
  - Sprint IDs: T2.6, T2.7, T8.1, T8.3.

- [ ] [P0] UI exposes callbacks/decisions without a runtime support matrix.
  `dns.response`, `hook.decision`, and generic `rewrite` need confirmation
  against actual enforcement.
  - Paths: `frontend/src/lib/models/settings-model.ts`,
    `frontend/src/lib/types/settings.ts`,
    `crates/capsem-core/src/net/policy_config/types.rs`,
    `crates/capsem-core/src/net/dns/server.rs`,
    `crates/capsem-core/src/net/mitm_proxy/policy_v2_http_hook.rs`,
    `crates/capsem-core/src/net/mitm_proxy/policy_v2_model.rs`,
    `crates/capsem-core/src/net/mitm_proxy/mcp_frame.rs`.
  - Proof: runtime support matrix in T8.1 and frontend validation tests in
    T2.7.
  - Sprint IDs: T2.6, T8.1, T8.2, T8.3.

- [x] [P0] Settings save/apply can present success while running VMs still use
  stale policy because reload failures are swallowed or not surfaced.
  - Paths: `frontend/src/lib/stores/settings.svelte.ts`,
    `frontend/src/lib/components/settings/McpSection.svelte`,
    `frontend/src/lib/stores/mcp.svelte.ts`,
    `crates/capsem-process/src/ipc.rs`,
    `crates/capsem-process/src/main.rs`.
  - Proof: UI/store tests now cover reload failure state and retry; E2E
    running-session apply proof remains T8.4/T10.3.
  - Sprint IDs: T2.7, T8.4, T10.3.
  - T8 audit: Mendel identified the missing SettingsPage component proof and
    all-sessions-stopped dismissal rule; T8 adds store/component coverage for
    affected session IDs, retry success, clear-on-change, and dismissal after
    all affected sessions stop.

- [x] [P1] Asset readiness and runtime/image truth were too optimistic.
  `NewTabPage.svelte` treated missing `assetHealth` as ready, About had
  hardcoded version/kernel/runtime rows, asset UI hid version/source/manifest
  state, and create flows did not show selected image/rootfs truth.
  - Paths: `frontend/src/lib/components/shell/NewTabPage.svelte`,
    `frontend/src/lib/components/shell/CreateSandboxDialog.svelte`,
    `frontend/src/lib/stores/vms.svelte.ts`,
    `frontend/src/lib/types/gateway.ts`, `frontend/src/lib/api.ts`,
    `crates/capsem-service/src/main.rs`,
    `crates/capsem-gateway/src/status.rs`.
  - Proof: asset-health unknown-state tests, create dialog tests, hardcoded
    runtime/kernel removal, and New Tab asset-unknown visual smoke are recorded.
    Full live service visual proof remains T10.3.
  - Sprint IDs: new T2/T8 asset-runtime truth task, T10.3.

- [x] [P1] Frontend has stale image/fork API assumptions. `getImages()` calls
  `/images`, service routes do not appear to expose `/images`, and
  `CreateSandboxDialog.svelte` has no image selector despite
  `ProvisionRequest.from`.
  - Paths: `frontend/src/lib/api.ts`,
    `frontend/src/lib/components/shell/CreateSandboxDialog.svelte`,
    `frontend/src/lib/types/gateway.ts`,
    `crates/capsem-service/src/main.rs`,
    `crates/capsem-service/src/api.rs`.
  - Proof: stale `getImages()` API and image selector affordance are removed or
    hidden for this release; T8 owns any later production image/fork selector.
  - Sprint IDs: new T2/T8 image/fork UI contract task.

- [x] [P1] Session creation hardcodes `2048 MB / 2 CPU`, bypassing backend
  settings defaults.
  - Paths: `frontend/src/lib/components/shell/CreateSandboxDialog.svelte`,
    `frontend/src/lib/stores/vms.svelte.ts`, `config/defaults.toml`,
    `config/defaults.json`.
  - Proof: frontend store/dialog tests prove create payloads omit CPU/RAM by
    default and send them only after explicit override.
  - Sprint IDs: new T2/T8 asset-runtime truth task.

- [x] [P1] Manual mock settings remain stale versus generated defaults.
  - Paths: `frontend/src/lib/mock-settings.ts`,
    `frontend/src/lib/mock-settings.generated.ts`,
    `scripts/generate_schema.py`, `config/defaults.toml`,
    `config/defaults.json`.
  - Proof: T2.1 uses generated/default-backed mocks, with
    `tests/test_config.py::TestGenerateDefaultsJsonConformance::test_mock_ts_not_stale`.
  - Sprint IDs: T2.1, T10.3.

## T2 Execution Audit, 2026-05-10

Agent: Boole (`019e12e6-3d40-70e1-b10e-3c9c4d09e6e1`)

Status: completed; no edits made by the agent. Focused store proof reported:
`pnpm --dir frontend exec vitest run src/lib/__tests__/settings-store.test.ts`
passed before implementation changes in this turn.

- [x] [P1] Pending policy entries were not first-class. Covered by T2.2:
  merge `pendingChanges` into `policyRuleEntries`, including `null` deletes,
  and test staged add/import/generated/delete visibility.
- [x] [P1] Rename/type change could orphan the old rule. Covered by T2.3:
  add a rename helper that stages `old_key: null` plus the new key, with tests
  for same-key edit, rename, type change, and rename plus type change.
- [x] [P1] Save success could hide reload/apply failure. Covered by T2.7/T8.4:
  add saved-not-applied state, retry, and reload-failure tests for save and
  preset apply.
- [x] [P1] Manual mocks were stale. Covered by T2.1: make
  `mock-settings.ts` a thin generated-default wrapper with drift proof.
- [x] [P2] Import/draft validation was too loose. Covered by T2.4: mirror
  backend callback bucket, enum, non-empty condition, rewrite, and header-name
  constraints in model/UI tests.
- [x] [P2] Generated rules were noisy. Covered by T2.5: suppress unchanged
  effective/staged generated candidates and count only changed candidates.
- [x] [P2] Unsupported hook/image surfaces remained exposed. Covered by
  T2.6/T2.8/T8.6: hide hook and `/images` surfaces until production support is
  proved.
- [x] [P2] Asset/create defaults were optimistic. Covered by T2.8: missing or
  unknown asset health must not read as ready, and create payloads must omit
  CPU/RAM unless the user explicitly overrides them.
- [x] [P3] Settings types were duplicated. Covered by T2.1: use one canonical
  settings type module and re-export from compatibility surfaces.

## Code Paths To Name In Sprint Docs

- Policy/settings UI:
  `frontend/src/lib/models/settings-model.ts`,
  `frontend/src/lib/components/settings/PolicyRulesSection.svelte`,
  `frontend/src/lib/stores/settings.svelte.ts`,
  `frontend/src/lib/components/settings/McpSection.svelte`,
  `frontend/src/lib/stores/mcp.svelte.ts`,
  `frontend/src/lib/types/settings.ts`, `frontend/src/lib/types.ts`,
  `frontend/src/lib/mock-settings.ts`,
  `frontend/src/lib/mock-settings.generated.ts`, `scripts/generate_schema.py`,
  `config/defaults.toml`, `config/defaults.json`.
- Frontend tests:
  `frontend/src/lib/models/__tests__/settings-model.test.ts`,
  `frontend/src/lib/__tests__/settings-store.test.ts`,
  `frontend/src/lib/__tests__/settings-export.test.ts`,
  `frontend/src/lib/__tests__/mcp-store.test.ts`,
  `frontend/src/lib/__tests__/api.test.ts`,
  new `frontend/src/lib/__tests__/policy-rules-section.test.ts`,
  and new VM asset/create dialog tests.
- E2E tests:
  `tests/capsem-e2e/test_policy_v2_http_dns_mitm.py`,
  `tests/capsem-e2e/test_model_policy_mitm.py`,
  `tests/capsem-e2e/test_framed_mcp_mitm.py::test_framed_guest_mcp_policy_reload_blocks_existing_connection`,
  `tests/capsem-install/test_asset_download.py`,
  `tests/capsem-install/test_auto_launch.py::test_auto_launch_missing_assets`,
  `tests/capsem-install/test_error_paths.py::test_missing_assets_dir`,
  `tests/capsem-build-chain/test_manifest_regen.py`.
