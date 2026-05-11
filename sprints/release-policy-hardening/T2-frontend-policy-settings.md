# T2: Frontend Policy Settings

## Objective

Make Policy V2 settings trustworthy in the desktop UI. Users must be able to
add, import, generate, rename, delete, review, save, and export policy rules
without invisible staged state or stale mock data. The UI must not expose
policy surfaces that the runtime cannot enforce in this release.

## Owned Files

- `frontend/src/lib/models/settings-model.ts`
- `frontend/src/lib/stores/settings.svelte.ts`
- `frontend/src/lib/components/settings/PolicyRulesSection.svelte`
- `frontend/src/lib/components/settings/SettingsSection.svelte`
- `frontend/src/lib/components/shell/NewTabPage.svelte`
- `frontend/src/lib/components/shell/CreateSandboxDialog.svelte`
- `frontend/src/lib/mock-settings.ts`
- `frontend/src/lib/mock-settings.generated.ts`
- `frontend/src/lib/types.ts`
- `frontend/src/lib/types/settings.ts`
- `frontend/src/lib/__tests__/*settings*`
- `frontend/src/lib/__tests__/policy-rules-section.test.ts`
- `frontend/src/lib/__tests__/session-runtime-truth.test.ts`
- `frontend/src/lib/models/__tests__/settings-model.test.ts`
- `frontend/package.json`
- `frontend/vitest.config.ts`
- `config/defaults.toml`
- `config/defaults.json`

## Findings

- [P1] `frontend/src/lib/mock-settings.ts` is manual/stale and tests do not use
  `frontend/src/lib/mock-settings.generated.ts`, which matches
  `config/defaults.json`.
- [P1] `PolicyRulesSection.svelte:106` records `editingKey`, but
  `stageDraft` at `PolicyRulesSection.svelte:131` stages only the new key and
  never stages `null` for the original key during rename/type change.
- [P1] `settings-model.ts:213` builds `policyRuleEntries` only from the
  effective `_policy` object, so newly staged/imported/generated rules are not
  visible in `PolicyRulesSection.svelte:35` until after save/reload.
- [P1] The frontend saves settings, replaces its local model, then swallows
  `/reload-config` failures in `settings.svelte.ts:99`, so running VMs can keep
  stale policy while the UI shows a clean saved state.
- [P2] `settings-model.ts:102` validates only primitive fields for imported
  policy rules. It accepts arbitrary callback/decision strings and
  incompatible type/callback pairs that the backend later rejects.
- [P2] `generatedPolicyRuleEntries` dedupes only within generated output. It
  does not suppress generated rules already effective or already staged
  unchanged, so `Stage all` can stay noisy.
- [P2] Policy tests cover model/store import/export paths but do not render
  `PolicyRulesSection.svelte`, all callbacks, rename/delete, or staged review.
- [P2] The UI exposes `hook` rules and `hook.decision` callbacks, but the
  production runtime does not load hook endpoints or call `PolicyHookClient`.
- [P2] There is no real component-test infrastructure for the Policy UI yet:
  no DOM/Svelte testing setup is present.
- [P3] `frontend/src/lib/types.ts` duplicates settings types from
  `frontend/src/lib/types/settings.ts`, and generated mocks import from
  `./types`.
- [P1] Asset readiness, image/fork UI, create defaults, and runtime status
  truth are currently only in swarm findings; they need an executable owner so
  the UI does not show missing assets or unsupported image routes as ready.

## Swarm Transfer Tracker

| Source | Priority | Owner task | Required transfer point | Required proof |
|---|---:|---|---|---|
| FD01 ui-policy-settings | P0 | T2.6, T2.7 | Hook policy remains exposed before T8 decides whether production hook dispatch/config ships. | Component tests prove unsupported hook rule/callback controls are hidden unless T8 ships them. |
| FD01 ui-policy-settings | P0 | T2.6, T8.6 | Callback/decision options need runtime support matrix for `dns.response`, `hook.decision`, and generic `rewrite`. | T8.6 matrix and frontend validation tests restrict options to proved runtime support. |
| FD01 ui-policy-settings | P0 | T2.7 | Settings save/apply can present success while running VMs keep stale policy. | Store/component tests show reload-failure saved-not-applied banner and retry behavior. |
| FD01 ui-policy-settings | P1 | T2.8 | Asset readiness and runtime/image truth were optimistic: missing `assetHealth` read ready, About had hardcoded runtime rows, and asset UI hid manifest/source state. | Unknown/missing asset state tests and visual proof show missing assets are not ready; hardcoded runtime/kernel rows are removed until a stamped runtime source exists. |
| FD01 ui-policy-settings | P1 | T2.8 | Frontend has stale image/fork API assumptions around `/images` and no image selector for `ProvisionRequest.from`. | Either `/images` plus selector/from flow ships with tests, or stale API/affordance is removed/hidden with tests. |
| FD01 ui-policy-settings | P1 | T2.8 | Session creation hardcodes `2048 MB / 2 CPU`, bypassing backend settings defaults. | Dialog/store tests prove create defaults come from settings/runtime source or are explicitly labeled overrides. |
| FD01 ui-policy-settings | P1 | T2.1 | Manual mock settings drift from generated defaults. | Generated/default-backed mock tests pass, including `tests/test_config.py::TestGenerateDefaultsJsonConformance::test_mock_ts_not_stale`. |
| FD11 verification-architecture | P1 | T2.8 | Frontend runtime/image truth needs an executable owner rather than buried under Policy settings. | T2.8 and T8.6 remain explicit until asset health, image/fork UI, create defaults, and service/gateway truth are proved or hidden. |
| FD14 swarm-transfer-closeout | P1 | T2.8 | Placeholder asset-runtime and image/fork owner references from FD14 must disappear. | `rg` finds no unresolved placeholder owner language outside historical finding docs. |

## Task List

### T2.1 Single Source for Settings Mocks

- [x] Make generated settings the base source for app mock mode and tests.
- [x] Keep hand-authored presets, issues, and policy fixtures only as a thin
  wrapper layered on generated defaults.
- [x] Add a drift test that fails when manual mock defaults diverge from
  generated `config/defaults.json`.
- [x] Decide one canonical settings type source and re-export from the other
  instead of duplicating fields.

### T2.2 Reviewable Pending Policy State

- [x] Merge pending `policy.*` changes into `policyRuleEntries`.
- [x] Apply pending `null` values as deletes in the visible review model.
- [x] Render pending additions/updates with a staged marker.
- [x] Render pending deletes as visible but dimmed until save/discard, or make
  deletion visible in an equivalent review surface.
- [x] Ensure dirty bar counts match visible staged policy changes.
- [x] Sort effective and pending entries consistently.

### T2.3 Atomic Rename and Type Change

- [x] Add a store/helper API that stages rename/type change as one batch:
  `old_key: null` plus `new_key: rule`.
- [x] Use that helper when `editingKey !== newKey`.
- [x] Ensure cancel/discard clears both sides of a rename batch.
- [x] Add coverage for edit same key, rename key, change type, and rename plus
  type change.

### T2.4 Import and Draft Validation

- [x] Validate callback enum and decision enum before staging imports.
- [x] Validate policy type/callback bucket compatibility.
- [x] Validate non-empty condition fields.
- [x] Validate rewrite rules: required rewrite fields for rewrite decisions and
  no rewrite fields on non-rewrite decisions.
- [x] Normalize or reject header arrays according to backend expectations.
- [x] Reject duplicate rule keys in import payloads before staging.
- [x] Prevent invalid rewrite drafts in `PolicyRulesSection.svelte`.

Coverage note: T2.4 validates callback buckets, decisions, condition presence,
rewrite target/value/capture rules, header names, duplicate import keys, and
unsupported draft combinations before local staging. Full CEL field/schema
validation remains backend-owned and is still part of T8 runtime proof.

### T2.5 Generated Rule UX

- [x] Suppress generated candidates that are already effective and unchanged.
- [x] Suppress generated candidates that are already staged unchanged.
- [x] Make `Stage all` count only new or changed candidates.
- [x] Add tests for generated single-stage and stage-all behavior.

### T2.6 Runtime-Truthful Surfaces

- [x] Until T8 records that configured external hook dispatch ships, remove
  `hook` from editable rule-type choices and remove `hook.decision` from
  editable callback choices. Do not show disabled hook controls in Settings as
  release UI.
- [x] If T8 records that hook dispatch ships, expose only the callback
  boundaries wired to production dispatch and covered by T8 E2E tests. T8.1
  records that configured external hook dispatch does not ship in `1.1.1778445002`.
- [x] Match visible callback options to the runtime enforcement map after T8 is
  decided.

Current release posture: editable Settings controls hide hook rules,
`hook.decision`, and unsupported `dns.response`. T8.6 records the final
production runtime support matrix for `1.1.1778445002`.

### T2.7 Component and Visual Coverage

- [x] Add `frontend/src/lib/__tests__/policy-rules-section.test.ts`.
- [x] Add the chosen Svelte DOM test harness explicitly:
  `@testing-library/svelte` plus `jsdom`, and configure Vitest so
  `policy-rules-section.test.ts` runs in `jsdom` while existing model/store
  tests remain unchanged.
- [x] Component tests: add rule, edit same key, rename, type change, delete,
  generated single-stage/stage-all, and unsupported callback/rule hiding.
- [ ] Visual verify Settings -> Policy in `just ui`: add/edit/delete/rewrite,
  generated/stage-all, save/discard dirty bar, import then review staged policy.

Visual status: the dev UI rendered successfully, and New Tab asset-unknown
state was captured at
`/var/folders/l5/jg8zh4215ll399vd5mcp9sp40000gn/T/chrome-devtools-mcp-ZwCoIv/screenshot.png`.
The full Settings -> Policy interaction path remains Gate A/T10 debt because
the local service was unavailable during the browser smoke.

### T2.8 Runtime and Image Truth

- [x] Add an explicit asset-health unknown state in UI models/stores instead
  of treating missing `assetHealth` as ready.
- [x] Make About/runtime/version display use stamped service/app data instead
  of hardcoded placeholders; hardcoded runtime/kernel rows are removed, and the
  remaining version row uses the app version stamp.
- [x] Decide with T8 whether `/images` and image/fork selection ship in this
  release.
- [x] If image selection ships, wire the UI to the production route and add a
  create dialog selector for the `--image`/`--from` path. T8.6 defers the
  user-selectable image/fork UI surface for `1.1.1778445002`.
- [x] If image selection does not ship, remove or hide stale `getImages()` and
  image/fork affordances from release UI.
- [x] Ensure create defaults come from the same settings/runtime source used by
  service/session creation.
- [x] Add tests for missing assets, unknown asset health, create defaults,
  resource override payloads, and unsupported route hiding.
- [ ] Visual verify that missing assets, unavailable image state, and service
  errors do not render as ready/success.

Current release posture: image selection is not exposed in the release UI, and
session creation omits CPU/RAM unless the user chooses the explicit override.
T8.6 defers any later production image/fork selector.

## Proof Matrix

| Category | Required proof |
|---|---|
| Unit/contract | settings model merges pending policy state and rejects malformed imports. |
| Component | `PolicyRulesSection.svelte` renders lifecycle states and callback options. |
| Functional | save/export/import round trips preserve adds, edits, renames, and deletes. |
| Adversarial | malformed callback, decision, bucket mismatch, invalid rewrite, duplicate keys are rejected before staging. |
| E2E/UI | `just ui` screenshots prove staged changes are visible and no settings text overlaps. |
| Runtime truth | asset readiness, image/fork controls, create defaults, and service status match production support. |
| Missing/deferred | hook and image/fork surfaces are hidden unless T8 wires production enforcement/support. |

## Verification

- [x] `cd frontend && pnpm --config.store-dir=/Users/elie/Library/pnpm/store/v10 run check`
- [x] `cd frontend && pnpm --config.store-dir=/Users/elie/Library/pnpm/store/v10 exec vitest run`
  (17 files, 381 tests passed).
- [x] `pnpm -C frontend test -- src/lib/__tests__/settings-store.test.ts src/lib/__tests__/settings-page-reload-banner.test.ts src/lib/__tests__/api.test.ts src/lib/__tests__/settings-export.test.ts src/lib/models/__tests__/settings-model.test.ts src/lib/__tests__/policy-rules-section.test.ts`
  (19 files, 388 tests passed after T8 reload-banner dismissal coverage).
- [x] `cd frontend && pnpm --config.store-dir=/Users/elie/Library/pnpm/store/v10 exec vitest run src/lib/__tests__/session-runtime-truth.test.ts`
  (5 tests passed, added after the T2.8 gap check).
- [ ] Coverage report: not run in T2; no coverage gate is required before the
  focused T10 frontend proof unless T10 adds an executable coverage command.
- [x] `cd frontend && pnpm --config.store-dir=/Users/elie/Library/pnpm/store/v10 run build`
- [x] `env UV_CACHE_DIR=/private/tmp/capsem-uv-cache uv run pytest tests/test_config.py::TestGenerateDefaultsJsonConformance::test_mock_ts_not_stale -q`
  (1 passed; sandbox run hit `psutil` permission denial, escalated rerun passed).
- [x] `git diff --check`
- [ ] `just ui`
- [ ] Chrome DevTools MCP: navigate to `http://localhost:5173`.
- [ ] Chrome DevTools MCP: list console messages for `error` and `warn`; expect
  zero before interaction.
- [ ] Screenshots: Settings -> Policy empty/default, add, edit same key,
  rename, type change, delete staged, import staged, generated single-stage,
  stage-all, save/discard dirty bar, reload-failure banner.
- [ ] Screenshots: New tab / create dialog asset-health unknown, missing
  assets, image/fork selector shipped or hidden, and create defaults.
- [ ] Chrome DevTools MCP: repeat console check; expect zero new
  errors/warnings.

Recorded browser smoke:

- [x] `astro dev --host 127.0.0.1 --port 5173` rendered New Tab with unknown
  asset health and disabled session-creation buttons.
- [x] Screenshot:
  `/var/folders/l5/jg8zh4215ll399vd5mcp9sp40000gn/T/chrome-devtools-mcp-ZwCoIv/screenshot.png`
- [ ] Full Settings -> Policy screenshot set remains Gate A/T10 debt because
  the local service was unavailable during this T2 browser smoke.

## Exit Criteria

- [x] No stale manual mock policy data is used by tests or mock mode.
- [x] Renaming/type-changing a policy rule cannot leave the old rule behind.
- [x] Staged new/imported/generated rules are visible before save.
- [x] Import validation mirrors backend policy constraints closely enough that
  bad imports do not stage locally.
- [x] Running-session reload failures are surfaced as saved-but-not-applied, not
  swallowed as success.
- [x] UI asset/image/runtime states are truthful in the implemented frontend
  contract; unsupported release surfaces are hidden, service-default creation is
  the default, and remaining runtime/image scope is explicitly owned by T8/T10.
- [ ] Full Gate A visual proof for Settings -> Policy and live service status is
  recorded in T10.
