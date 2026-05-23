# S16 - Profile UI

## Goal

Make signed/catalog profiles, profile revisions, package contracts, asset
readiness, and profile-backed VM creation first-class in the UI.

This is a release-blocking usable surface for the Profile V2 bedrock. It is not
marketing polish and not a later workbench sprint. Operators must be able to use
the new HTTP/UDS endpoint contract from the UI: select a profile, see its
revision/assets/rules, create a VM from it, inspect existing VM profile state,
and operate runtime enforcement/detection overlays without falling back to raw
requests.

## Tasks

- Add profile selector.
- Add catalog/profile list with the canonical `ProfileRevisionStatus` enum:
  `active`, `deprecated`, and `revoked`. Do not display `removed` as a status;
  absent revisions are simply not offered.
- Add profile revision view: installed revision, catalog current revision,
  update availability, binary compatibility, payload verification state.
- Add create, fork, delete flows for user-authored profiles, while clearly
  separating catalog-installed corp/base profiles from editable user profiles.
- Show icon, name, description, best-for, type, version/revision.
- Show package/tool contract and VM asset readiness for the selected profile.
- Add General, Appearance, AI Providers, MCP & Connectors, Skills, VM, Security.
- Show profile-owned enforcement packs and detection packs as separate
  sections. Enforcement controls link to blocking-capable CEL rule editing and
  enforcement backtest. Detection controls link to Sigma/detection pack
  validation, detection backtest, stats, findings, and hunt.
- Make VM/session launch use an explicit selected profile id and resolved
  revision. The create flow must surface first-use asset download progress and
  block revoked/incompatible profiles.
- Show existing VM bindings: profile id/revision, package contract hash, pinned
  asset hashes, and drift/deprecation/revocation warnings.
- Add runtime enforcement/detection operation panels backed by the S08b route
  families: list, validate, install/update/delete runtime overlays where
  allowed, show read-only profile/corp/user-owned rules, show stats, and start
  backtest/hunt flows using the service result shape.
- Backtest UI defaults use the service result shape: summary counts plus up to
  100 matched events, deduped by evidence signature, with event refs and full
  local matched evidence. Redacted views are explicit export/support-bundle
  flows.

## Coverage Ledger

- Implemented slice: Settings -> Policy now has a typed Security Engine health
  panel backed by `/debug/report`. It shows enforcement/detection rule counts,
  enabled/compiled/error state, match/finding totals, runtime-rule store state,
  and confirm resolver availability. Missing or malformed debug-report security
  blocks fail closed with an explicit unavailable state instead of throwing.
- Implemented slice: Settings -> Profiles now has a typed catalog panel backed
  by `/profiles/catalog`. It renders profile ids, installed/current revision
  drift, per-revision hashes, and only the canonical lifecycle statuses:
  `active`, `deprecated`, and `revoked`.
- Implemented slice: profile selection now uses a profile-native
  `POST /profiles/{id}/select` service route. `/profiles/catalog` returns the
  selected `default_profile`, and Settings -> Profiles shows the selected
  profile, can select non-revoked profiles, and disables revoked selections.
- Implemented slice: quick-session and customize-session VM create now include
  the service-reported profile id and resolved revision when asset health
  exposes them. The customize dialog also displays the active
  `profile@revision` so operators can see which profile will back the VM
  before launch.
- Implemented slice: the session list now shows each VM's profile id,
  revision, and typed status (`current`, `needs_update`, `deprecated`,
  `revoked`, `corrupted`, or `unknown`). VMs without a profile pin render as
  corrupted instead of looking valid.
- Implemented slice: the Sessions screen now shows ready profile asset
  provenance from `/status`: active profile revision, architecture, asset
  version, profile payload hash, and each profile-declared VM asset's
  source/hash/size.
- Unit/contract: profile UI model tests for all `ProfileRevisionStatus` enum
  values, revisions, package/tool contracts, asset readiness, VM pin fields,
  enforcement-pack summaries, detection-pack summaries, and backtest result
  rows.
- Unit/contract completed: `security-engine-health-section.test.ts` covers the
  typed Security Engine health projection, manual refresh, and missing security
  block fallback. Existing runtime-rule panel and debug-copy tests were rerun.
- Unit/contract completed: `profile-catalog-section.test.ts` covers profile
  catalog rendering, update/not-installed states, the
  `active`/`deprecated`/`revoked` enum display, the absence of `removed`, and
  manual refresh. `api.test.ts` covers the typed profile catalog and revision
  routes.
- Unit/contract completed: service tests cover profile-native selection and
  catalog `default_profile` reporting; frontend tests cover `selectProfile`,
  selected badges, successful selection, and disabled revoked profile
  selection.
- Unit/contract completed: session runtime truth tests cover quick-session and
  customize-session create requests carrying `profile_id` and
  `profile_revision` from asset health, while still omitting CPU/RAM in
  service-default mode.
- Unit/contract completed: session runtime truth tests cover session-list
  profile identity/status rendering and the missing-profile corrupted marker.
- Unit/contract completed: session runtime truth tests cover ready profile
  asset provenance rendering, including profile payload hash and per-asset
  source/hash/size rows.
- Functional: create/fork/delete/select tests; update/install catalog revision;
  profile-backed VM create with asset readiness states; enforcement/detection
  runtime overlay list/validate/install/delete/stats/backtest/hunt flows through
  the HTTP gateway.
- Adversarial: locked/forbidden profile actions, revoked profile, incompatible
  profile revision, stale catalog rollback warning, asset download failure,
  interrupted download retry, and invalid/missing VM pin display.
- E2E/VM: launch session with selected profile revision and verified assets.
- Telemetry: UI links to status/debug provenance for profile revision, asset
  verification failures, enforcement matches, detection findings, and rule
  stats.
- Telemetry completed: the first debug provenance surface now renders
  `/debug/report` runtime Security Engine counters in the Policy UI.
- Performance: profile switching remains responsive and does not trigger network
  fetches or hash scans on every selection change.
- Visual/build proof: Settings -> Policy was opened in the local Astro UI and
  screenshot-checked; the live dev gateway returned a debug report without a
  security block, proving the explicit unavailable fallback path. Production
  frontend build passed.
- Visual/build proof: Settings -> Profiles was opened in the local Astro UI and
  screenshot-checked; the live dev gateway returned `404` for the catalog route,
  proving the explicit gateway-error fallback path. Production frontend build
  passed.
- Visual/build proof: Settings -> Profiles was also screenshot-checked with a
  browser-side catalog fixture so selected, update-available, active,
  deprecated, revoked, installed, and disabled-selection states were visible in
  the actual UI layout.
- Visual/build proof: Customize Session was screenshot-checked with a
  browser-side ready asset-health fixture so the profile badge and create flow
  layout were visible in the actual UI.
- Visual/build proof: the session list was screenshot-checked with a
  browser-side `/status` fixture showing current, needs-update, and missing-pin
  corrupted profile states in the actual table layout.
- Visual/build proof: the profile asset readiness panel was screenshot-checked
  with a browser-side `/status` fixture showing the active profile revision,
  payload hash, and profile asset rows in the actual Sessions layout.
