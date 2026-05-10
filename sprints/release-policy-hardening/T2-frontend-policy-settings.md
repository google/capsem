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

## Task List

### T2.1 Single Source for Settings Mocks

- [ ] Make generated settings the base source for app mock mode and tests.
- [ ] Keep hand-authored presets, issues, and policy fixtures only as a thin
  wrapper layered on generated defaults.
- [ ] Add a drift test that fails when manual mock defaults diverge from
  generated `config/defaults.json`.
- [ ] Decide one canonical settings type source and re-export from the other
  instead of duplicating fields.

### T2.2 Reviewable Pending Policy State

- [ ] Merge pending `policy.*` changes into `policyRuleEntries`.
- [ ] Apply pending `null` values as deletes in the visible review model.
- [ ] Render pending additions/updates with a staged marker.
- [ ] Render pending deletes as visible but dimmed until save/discard, or make
  deletion visible in an equivalent review surface.
- [ ] Ensure dirty bar counts match visible staged policy changes.
- [ ] Sort effective and pending entries consistently.

### T2.3 Atomic Rename and Type Change

- [ ] Add a store/helper API that stages rename/type change as one batch:
  `old_key: null` plus `new_key: rule`.
- [ ] Use that helper when `editingKey !== newKey`.
- [ ] Ensure cancel/discard clears both sides of a rename batch.
- [ ] Add coverage for edit same key, rename key, change type, and rename plus
  type change.

### T2.4 Import and Draft Validation

- [ ] Validate callback enum and decision enum before staging imports.
- [ ] Validate policy type/callback bucket compatibility.
- [ ] Validate non-empty condition fields.
- [ ] Validate rewrite rules: required rewrite fields for rewrite decisions and
  no rewrite fields on non-rewrite decisions.
- [ ] Normalize or reject header arrays according to backend expectations.
- [ ] Reject duplicate rule keys in import payloads before staging.
- [ ] Prevent invalid rewrite drafts in `PolicyRulesSection.svelte`.

### T2.5 Generated Rule UX

- [ ] Suppress generated candidates that are already effective and unchanged.
- [ ] Suppress generated candidates that are already staged unchanged.
- [ ] Make `Stage all` count only new or changed candidates.
- [ ] Add tests for generated single-stage and stage-all behavior.

### T2.6 Runtime-Truthful Surfaces

- [ ] Until T8 records that configured external hook dispatch ships, remove
  `hook` from editable rule-type choices and remove `hook.decision` from
  editable callback choices. Do not show disabled hook controls in Settings as
  release UI.
- [ ] If T8 records that hook dispatch ships, expose only the callback
  boundaries wired to production dispatch and covered by T8 E2E tests.
- [ ] Match visible callback options to the runtime enforcement map after T8 is
  decided.

### T2.7 Component and Visual Coverage

- [ ] Add `frontend/src/lib/__tests__/policy-rules-section.test.ts`.
- [ ] Add the chosen Svelte DOM test harness explicitly:
  `@testing-library/svelte` plus `jsdom`, and configure Vitest so
  `policy-rules-section.test.ts` runs in `jsdom` while existing model/store
  tests remain unchanged.
- [ ] Component tests: add rule, edit same key, rename, type change, delete,
  import, generated single-stage, generated stage-all, every supported
  callback option.
- [ ] Visual verify Settings -> Policy in `just ui`: add/edit/delete/rewrite,
  generated/stage-all, save/discard dirty bar, import then review staged policy.

### T2.8 Runtime and Image Truth

- [ ] Add an explicit asset-health unknown state in UI models/stores instead
  of treating missing `assetHealth` as ready.
- [ ] Make About/runtime/version display use stamped service/app data instead
  of hardcoded placeholders.
- [ ] Decide with T8 whether `/images` and image/fork selection ship in this
  release.
- [ ] If image selection ships, wire the UI to the production route and add a
  create dialog selector for the `--image`/`--from` path.
- [ ] If image selection does not ship, remove or hide stale `getImages()` and
  image/fork affordances from release UI.
- [ ] Ensure create defaults come from the same settings/runtime source used by
  service/session creation.
- [ ] Add tests for missing assets, unknown asset health, create defaults,
  image selector visibility, and unsupported route hiding.
- [ ] Visual verify that missing assets, unavailable image state, and service
  errors do not render as ready/success.

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

- [ ] `cd frontend && pnpm run check`
- [ ] `cd frontend && npx vitest run src/lib/models/__tests__/settings-model.test.ts`
- [ ] `cd frontend && npx vitest run src/lib/__tests__/settings-store.test.ts`
- [ ] `cd frontend && npx vitest run src/lib/__tests__/settings-export.test.ts`
- [ ] `cd frontend && npx vitest run src/lib/__tests__/policy-rules-section.test.ts`
- [ ] `cd frontend && npx vitest run --coverage`
- [ ] `cd frontend && pnpm run build`
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

## Exit Criteria

- [ ] No stale manual mock policy data is used by tests or mock mode.
- [ ] Renaming/type-changing a policy rule cannot leave the old rule behind.
- [ ] Staged new/imported/generated rules are visible before save.
- [ ] Import validation mirrors backend policy constraints closely enough that
  bad imports do not stage locally.
- [ ] Running-session reload failures are surfaced as saved-but-not-applied, not
  swallowed as success.
- [ ] UI asset/image/runtime states are truthful; unsupported release surfaces
  are hidden or documented as deferred.
