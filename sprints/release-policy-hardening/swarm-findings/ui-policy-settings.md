# UI Policy Settings Findings

Status: completed, pending transfer into T2/T8/T10.

Agent: Jason (`019e1263-534b-7702-864a-ca1f7b3a4f74`)

## Scope

- Frontend Settings support for Policy V2.
- Staged pending state for add/edit/rename/delete/import/generated rules.
- Hook-control truthfulness if configured hook dispatch does not ship.
- Visual and component coverage.
- UI surfaces that show image/asset/runtime policy state.

## Findings

- [ ] [P0] Hook policy remains exposed in Settings UI even though T8 has not
  resolved whether production hook dispatch/config ships. Either T8 must ship
  hook runtime or T2.6 must hide hook controls.
  - Paths: `frontend/src/lib/components/settings/PolicyRulesSection.svelte`,
    `frontend/src/lib/models/settings-model.ts`.
  - Proof: component tests for hidden unsupported callbacks/decisions, plus
    T8 shipped-scope proof.
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

- [ ] [P0] Settings save/apply can present success while running VMs still use
  stale policy because reload failures are swallowed or not surfaced.
  - Paths: `frontend/src/lib/stores/settings.svelte.ts`,
    `frontend/src/lib/components/settings/McpSection.svelte`,
    `frontend/src/lib/stores/mcp.svelte.ts`,
    `crates/capsem-process/src/ipc.rs`,
    `crates/capsem-process/src/main.rs`.
  - Proof: UI/store tests for reload failure state and E2E running-session
    apply proof.
  - Sprint IDs: T2.7, T8.4, T10.3.

- [ ] [P1] Asset readiness and runtime/image truth are too optimistic.
  `NewTabPage.svelte` treats missing `assetHealth` as ready, About hardcodes
  version/kernel/runtime, asset UI hides version/source/manifest state, and
  create flows do not show selected image/rootfs truth.
  - Paths: `frontend/src/lib/components/shell/NewTabPage.svelte`,
    `frontend/src/lib/components/shell/CreateSandboxDialog.svelte`,
    `frontend/src/lib/stores/vms.svelte.ts`,
    `frontend/src/lib/types/gateway.ts`, `frontend/src/lib/api.ts`,
    `crates/capsem-service/src/main.rs`,
    `crates/capsem-gateway/src/status.rs`.
  - Proof: asset-health unknown-state tests, create dialog tests, and visual
    proof that missing assets are not shown as ready.
  - Sprint IDs: new T2/T8 asset-runtime truth task, T10.3.

- [ ] [P1] Frontend has stale image/fork API assumptions. `getImages()` calls
  `/images`, service routes do not appear to expose `/images`, and
  `CreateSandboxDialog.svelte` has no image selector despite
  `ProvisionRequest.from`.
  - Paths: `frontend/src/lib/api.ts`,
    `frontend/src/lib/components/shell/CreateSandboxDialog.svelte`,
    `frontend/src/lib/types/gateway.ts`,
    `crates/capsem-service/src/main.rs`,
    `crates/capsem-service/src/api.rs`.
  - Proof: either implement `/images` plus selector/from flow or remove/defer
    stale frontend API and docs.
  - Sprint IDs: new T2/T8 image/fork UI contract task.

- [ ] [P1] Session creation hardcodes `2048 MB / 2 CPU`, bypassing backend
  settings defaults.
  - Paths: `frontend/src/lib/components/shell/CreateSandboxDialog.svelte`,
    `frontend/src/lib/stores/vms.svelte.ts`, `config/defaults.toml`,
    `config/defaults.json`.
  - Proof: frontend store/dialog tests proving create defaults come from
    settings or are explicitly labeled as overrides.
  - Sprint IDs: new T2/T8 asset-runtime truth task.

- [ ] [P1] Manual mock settings remain stale versus generated defaults.
  - Paths: `frontend/src/lib/mock-settings.ts`,
    `frontend/src/lib/mock-settings.generated.ts`,
    `scripts/generate_schema.py`, `config/defaults.toml`,
    `config/defaults.json`.
  - Proof: T2.1 requires generated/default-backed mocks only, with
    `tests/test_config.py::TestGeneratedConfigSync::test_mock_settings_not_stale`.
  - Sprint IDs: T2.1, T10.3.

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
