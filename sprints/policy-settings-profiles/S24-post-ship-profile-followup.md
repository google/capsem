# S24 - Post-Ship Profile V2 Follow-Up

## Goal

Clean up the Profile V2 work left after the bedrock release shipped.

This is the single active Profile V2 follow-up sprint. It replaces scattered
release-hit-list proof notes and older "done for bedrock" caveats with one
operational queue.

## Scope

### A. Installed Product Proof

Close the shipped-but-needs-proof items from `release-hit-list.md`:

- Package/UI waits for setup, service, and gateway readiness before opening.
- Onboarding, Settings Profiles, and dashboard profile cards use installed
  profiles and do not surface catalog emptiness as a user error.
- Dashboard/session creation starts from visible profile cards.
- UI does not show offline while service, gateway, tray, and CLI are healthy.
- `capsem run "echo test"` and `capsem shell` work immediately after install.
- Interrupted or repeated installs leave profile metadata and asset hashes
  coherent.
- Settings opens against the Profile V2 `/settings` envelope.
- Profile cards do not advertise unprovisionable profiles.

### B. Remaining Product Fixes

Fix or deliberately defer the still-open product issues:

- RHB-006: profile auth 401 during setup/onboarding.
- RHB-011: confusing developer `just install` output after setup completes.
- RHB-012: verify 4 CPU / 8 GB RAM / 8 active VM defaults and active-only
  counting.
- RHB-013: old credential scan clarity. Coordinate with
  `../credential-pipeline/`; do not implement full S10 brokerage here.
- RHB-017: installed VM proof for Gemini credential projection and wrapper
  defaults.
- RHB-018: installed VM proof that live `/status` and toolbar counters reflect
  model tokens/cost.

### C. Board Reconciliation

- Mark `release-hit-list.md` items as closed, migrated, or explicitly deferred.
- Reconcile stale S08b/S08d/S11/S16/S18 wording so the active board does not
  imply pre-release work is still in progress.
- Keep post-bedrock product expansions in their own lanes.

## Non-Goals

- No S10 credential release/brokerage into sessions.
- No full S12 OpenTelemetry/export/dashboard polish.
- No S16a timeline/workbench implementation.
- No Linux reboot.
- No service split refactor.
- No resurrection of retired dashboard, forensics, audit, or frontend boards.

## Tasks

- [ ] T0: Installed-state inventory. Record current installed version, service
      status, gateway status, profile list, profile catalog state, and app
      startup state.
- [ ] T1: Installed UI proof. Onboarding, Settings Profiles, dashboard profile
      cards, Settings page, and offline-state recovery.
- [ ] T2: Installed CLI/VM proof. `capsem status`, `capsem run "echo test"`,
      `capsem shell`, profile asset coherence, and package hook readiness.
- [ ] T3: Remaining product fixes. RHB-006, RHB-011, RHB-012, RHB-013.
- [ ] T4: VM/provider proof. Gemini env/wrapper and live token/cost counters.
- [ ] T5: Board closeout. Update `release-hit-list.md`, `NOW.md`, `MASTER.md`,
      and this file with proof and explicit deferrals.

## Coverage Ledger

- Unit/contract: focused tests for any code changes.
- Functional: installed app/profile/settings/dashboard flows.
- Adversarial: stale auth token, service restart, missing signed catalog
  revision, interrupted install, and profile asset mismatch.
- E2E/VM: installed `capsem run`, `capsem shell`, Gemini env/wrapper proof, and
  live metrics proof.
- Telemetry: `/status`, toolbar counters, logs/debug breadcrumbs for installed
  sessions.
- Performance: not primary.
- Missing/deferred: S10, S12, S16a, S19a/S19b, S20/S21/S22/S23.
