# Profile V2 Release Hit List

Date: 2026-05-24
Mode: historical release debug loop. The bedrock release shipped; active
post-ship work is migrated to
[S24 - Post-Ship Profile V2 Follow-Up](S24-post-ship-profile-followup.md).
Every remaining item below is evidence for S24, not a separate active sprint.

## Rules

- No memory-only tracking. New release bugs go here before or while fixing.
- A bug is closed only when the reproduction is understood and a test or install/UI proof is named.
- Product usability beats internal correctness. If CLI says healthy but UI says broken, the product is broken.
- Signed profile/catalog/admin internals are not allowed to leak into first-run user copy unless the screen is explicitly an admin/debug surface.

## P0 Release Blockers

### RHB-001 - Package/UI opens before install is truly ready

- Status: fixed in repo, needs package reinstall proof.
- Report: UI appeared while `just install` was still running setup/gateway/smoke work and showed no profile in red.
- Evidence: macOS `postinstall` previously opened `Capsem.app` after setup but before explicit service UDS and gateway health checks.
- Fix landed: `scripts/pkg-scripts/postinstall` now waits for service `/list` over UDS and gateway `/health` before opening the app; fails loudly instead of opening early.
- Proof so far: package script tests and shell syntax passed.
- Close proof needed: installed package run proves the app does not appear until setup + service + gateway are ready.

### RHB-002 - Onboarding profile select can show empty/red despite installed profiles

- Status: fixed in repo, needs installed UI proof.
- Report: Profile step showed "No profiles" / red error, then stayed empty after navigating back.
- Reproduction: installed service has usable profiles via `GET /profiles`, but `GET /profiles/catalog` returns `manifest_present: false` and `profiles: []` when no enterprise catalog is configured.
- Root cause: onboarding `PreferencesStep` used `/profiles/catalog`, which is the signed remote/catalog lifecycle surface, not the installed usable profile list.
- Fix landed: onboarding profile selection now uses `listProfiles()` / `GET /profiles`.
- Proof so far: `pnpm exec vitest run src/lib/__tests__/onboarding-preferences-step.test.ts` passed.
- Close proof needed: installed UI pass after rebuild.

### RHB-003 - Settings Profiles page shows catalog emptiness instead of installed profiles

- Status: fixed in repo, needs installed UI proof.
- Report: after returning to profile screen, profile select/list is empty even though CLI status and `/profiles` show installed profiles.
- Reproduction: `GET /profiles/catalog` currently returns no manifest/profiles on local package installs; `GET /profiles` returns base profiles with asset status.
- Root cause: Settings `ProfileCatalogSection` is catalog-first and treats missing enterprise manifest as "No profile catalog installed", which is wrong for the user-facing profiles screen.
- Fix landed: Settings Profiles now renders installed profiles from `GET /profiles`; signed catalog state is optional admin context and no longer drives the empty state.
- Proof so far: `pnpm exec vitest run src/lib/__tests__/profile-catalog-section.test.ts src/lib/__tests__/onboarding-preferences-step.test.ts` passed.
- Close proof needed: installed UI pass after rebuild.

### RHB-004 - New Session flow does not let the user click a profile

- Status: fixed in repo, needs installed UI proof.
- Report: "new session makes no sense -- we need to click on profile."
- Previous behavior: `CreateSandboxDialog` launched against `vmStore.assetHealth.profile_id` and the dashboard exposed generic "Quick Session" / "Customize Session" actions.
- Product contract: user starts a session from a visible profile choice, with icon/name/description and blocked state if unusable.
- Fix landed: dashboard now renders profile cards at the top from `GET /profiles`; clicking a card provisions with that `profile_id`/revision; the existing session lists remain below. Generic quick-session language is removed.
- Proof so far: `pnpm exec vitest run src/lib/__tests__/session-runtime-truth.test.ts src/lib/__tests__/profile-catalog-section.test.ts src/lib/__tests__/onboarding-preferences-step.test.ts` passed and `pnpm run check` passed.
- Close proof needed: installed UI pass after rebuild.

### RHB-005 - UI says offline while CLI/service are healthy

- Status: fixed in repo, still needs installed-app proof.
- Report: dashboard said Capsem offline while tray/service and `capsem status` said running/ok.
- Evidence: local logs showed a transient event-websocket/health miss flipped the UI offline while `/status`, service, gateway, tray, and CLI were healthy.
- Fix landed: frontend API retries `init()` before returning synthetic offline status, and the gateway store now confirms `/status` before marking an already-connected app disconnected after a health miss.
- Proof so far: `pnpm exec vitest run src/lib/__tests__/gateway-store.test.ts src/lib/__tests__/api.test.ts`, `pnpm run check`, and `just build-ui` passed.
- Close proof needed: installed UI opened after package hook shows online when `capsem status` is ready; add/keep frontend API regression.

### RHB-006 - Profile auth 401 during setup/onboarding

- Status: open, needs reproduction from logs/UI path.
- Report: "Profile ... API error 401: {\"error\":\"unauthorized\"}".
- Suspected root: UI made authenticated profile request before token refresh or while gateway token rotated during service restart.
- Related code: `_authFetch` retries once on 401, but first-run service restart/UI launch timing may still race.
- Close proof needed: frontend/API test for profile request after stale token; installed smoke that restarts service and profile screen recovers without surfacing 401.

### RHB-007 - `capsem run` / `capsem shell` must work immediately after install

- Status: fixed in repo and locally verified, keep in release gate.
- Report: `capsem shell` and `capsem run "echo test"` failed with `pin VM profile/package/assets`.
- Root causes fixed: duplicate runtime profile install and profile sidecar/catalog mismatch after install.
- Proof so far: focused Rust tests and local `capsem run "echo test"` passed.
- Close proof needed: include `capsem run "echo test"` in final installed release proof after next reinstall.

### RHB-008 - Install can leave profile hashes incoherent if interrupted

- Status: fixed in repo and locally repaired, keep in release gate.
- Report: `initrd.img` hash mismatch after reinstall attempt.
- Root cause: `just install` repacked live symlinked assets before package install completed, then old profile metadata remained active.
- Fix landed: `_pack-initrd` moved into install body with pre-package profile metadata repair; profiles are no longer restored over packaged base profiles.
- Close proof needed: next `just install` from clean-ish installed state produces coherent profile revision/assets and `capsem run` passes.

## P1 Product Polish / Usability

### RHB-009 - Profile cards look like raw asset/debug cards

- Status: fixed in primary UI surfaces, needs installed UI proof.
- Report: profile assets "look like shit"; raw asset list belongs in an info modal, not the primary profile picker.
- Fix landed: Settings Profiles and dashboard session creation cards show icon, name, description, best-for, selected/ready/blocked badges, and do not expose missing asset paths inline.
- Close proof needed: screenshot/visual test of profile selection surface.

### RHB-010 - First-run copy still uses VM/readiness jargon

- Status: partially fixed, needs installed UI pass.
- Report: users know sessions/profiles, not "VM readiness"; welcome should be simple.
- Fix landed: Welcome/Ready copy simplified and removed "What's New" and VM Assets from onboarding.
- Remaining risk: inline degraded-state asset warning still exists when the service reports a setup problem; primary dashboard/session creation no longer exposes raw VM asset provenance or an asset modal.
- Close proof needed: installed onboarding walkthrough screenshot/pass.

### RHB-011 - Dev install output is confusing after setup complete

- Status: open.
- Report: after "Setup complete", install continues with guest DNS/HTTPS and pruning; user unsure what it is.
- Diagnosis: this is the `just install` developer release gate, not the product package hook.
- Fix direction: label phases clearly as "Release smoke: guest DNS/HTTPS" and "Developer cleanup"; keep package hook quiet.
- Close proof needed: inspect `just install` output after relabel.

### RHB-012 - VM defaults copy/resources should be 4 CPU, 8 GB RAM, 8 active VMs

- Status: partially done.
- Report: 4/4/10 looked arbitrary; preferred 4 x 8 x 8 and active VMs should count active, not paused.
- Current onboarding display: 4 CPU, 8 GB, 8 active VMs.
- Remaining work: verify actual service defaults match product copy and counts are active-only; auto resource detection deferred to polish sprint unless it blocks release.
- Close proof needed: config/defaults plus UI copy test.

### RHB-013 - Credential scan may not surface old credentials

- Status: open.
- Report: old credentials not picked up; acceptable if breaking, but scan behavior must be clear.
- Fix direction: verify detection logs/settings snapshot and onboarding provider status; decide whether to rescan, explain breaking change, or add migration note.
- Close proof needed: provider detection test/log proof for GitHub/Google/Anthropic/OpenAI paths.

## P2 Engineering Hygiene / Release Artifacts

### RHB-014 - Simulated install failed when assets source equals install destination

- Status: fixed.
- Root cause: `assets` symlinked to `~/.capsem/assets`, making source/destination identical for `cp`.
- Fix landed: `simulate-install.sh` skips same-file copies.
- Proof: package script test passed.

### RHB-015 - Package hook setup is mandatory, not optional

- Status: fixed, keep in release proof.
- Report: install/package post hook must run setup; manual setup after install is not acceptable.
- Fix landed: package hooks fail if target user/setup cannot run; `just install` reruns setup after preserving user settings.
- Proof so far: package script tests passed.
- Close proof needed: final package/install gate.


### RHB-016 - Settings page crashes with undefined object error

- Status: fixed in repo, needs installed UI proof.
- Report: Settings returns `TypeError: undefined is not an object (evaluating 'e')`.
- Evidence: installed Tauri log reports `Failed to load settings: _buildIndexes@... SettingsModel.load`, so the crash is in the frontend settings model index construction after receiving settings data.
- Root cause: the Profile V2 service contract intentionally returns `profile_presets`, `effective_rules`, and `settings_profiles` from `/settings`; the frontend model still treated legacy `tree`, `issues`, and `presets` as mandatory and iterated `undefined`.
- Fix landed: `SettingsModel` now explicitly normalizes the Profile V2 envelope, maps `effective_rules` into policy state, maps `profile_presets` into UI presets, and defaults absent legacy arrays to empty typed arrays. MCP policy extraction now uses the same optional tree/effective-rules contract.
- Proof so far: `pnpm exec vitest run src/lib/__tests__/api.test.ts src/lib/models/__tests__/settings-model.test.ts src/lib/__tests__/settings-page-reload-banner.test.ts src/lib/__tests__/settings-debug-report.test.ts src/lib/__tests__/settings-store.test.ts src/lib/__tests__/profile-catalog-section.test.ts src/lib/__tests__/session-runtime-truth.test.ts src/lib/__tests__/onboarding-preferences-step.test.ts` passed; `pnpm run check` passed.
- Close proof needed: rebuild embedded UI/app and verify installed Settings screen opens.


### RHB-017 - Gemini credential and guest parameter injection broken

- Status: fixed in repo, needs installed VM proof.
- Report: rerunning the wizard and adding a Gemini key did not make Gemini work in the VM; manually forcing Gemini auth inside the VM worked, but the expected default YOLO/bypass parameters were missing.
- Root cause: the wizard saved `google-api-key` into Profile V2 service settings, but VM-effective settings did not project enabled provider credential refs into guest env, so `capsem-process` had no `GEMINI_API_KEY` to inject. Separately, Gemini YOLO mode was only a `/root/.bashrc` alias, which does not cover non-interactive exec or direct process launches.
- Fix landed: Profile V2 effective settings now carry a typed `credential_env` projection for enabled provider credential refs (`google-api-key` -> `GEMINI_API_KEY`, without also setting `GOOGLE_API_KEY`). `capsem-process` merges that map into boot env and injects a `/root/.local/bin/gemini` wrapper that defaults real Gemini invocations to `--yolo` without relying on aliases.
- Proof so far: `cargo test -p capsem-service handle_upsert_credential -- --nocapture`, `cargo test -p capsem-process mcp_runtime -- --nocapture`, `cargo check -p capsem-core -p capsem-process -p capsem-service`, and `uv run python -m py_compile guest/artifacts/diagnostics/test_ai_cli.py` passed.
- VM smoke note: `just exec` rebuilt, repacked, and signed the dev binaries, then stopped before boot because the installed service still owned the socket (`/Users/elie/.capsem/bin/capsem-service`, PID 86632). Do the final VM smoke after an intentional reinstall/restart, not by silently killing the user's active UI test service.
- Close proof needed: installed/rebuilt VM smoke showing `GEMINI_API_KEY` is present, `GOOGLE_API_KEY` is absent, `type -P gemini` resolves to `/root/.local/bin/gemini`, and the wrapper includes `--yolo`.


### RHB-018 - VM header cost/token counters show zero while Stats tab works

- Status: fixed in repo, needs installed VM proof after rebuild/restart.
- Report: the top VM UI cost/token counters show zero, while the Stats tab shows real values.
- Root cause: the toolbar reads the `/status` VM summary, which is backed by capsem-process's live in-memory metrics snapshot. Stats reads `model_calls` from `session.db`. The logger seeded model metrics from existing `model_calls` at process startup, but live `WriteOp::ModelCall` rows did not increment the accumulator, so a running VM could show zero in the header while DB-backed Stats was correct.
- Fix landed: VM-scoped `ModelCall` writes now update live model request/token/cost counters; host-scoped model calls remain excluded from VM accounting. The toolbar binding also has a regression that renders non-zero counters from a live VM summary.
- Counter coverage: dedicated memory-counter tests now cover HTTPS/HTTP request totals, allow/warn/deny/error buckets, and bytes; DNS totals, allow/warn/deny/rewrite/error buckets; MCP invocation buckets; file read/write/create/delete/restore/error buckets and bytes; process exec/audit/error buckets; model request/token/cost counters with host attribution excluded; and security ask/rewrite/throttle/block/error/detection counters.
- Proof so far: `cargo test -p capsem-logger writer_metrics_snapshot_counts_live_vm_model_call_rows -- --nocapture`, `cargo test -p capsem-logger metrics_snapshot -- --nocapture` (11 focused memory-counter tests), full `cargo test -p capsem-logger` (114 unit tests + 128 roundtrip tests), `cargo check -p capsem-logger -p capsem-process -p capsem-service`, `pnpm exec vitest run src/lib/__tests__/session-runtime-truth.test.ts`, `pnpm run check`, `cargo fmt --check`, `git diff --check`, and `just build-ui` passed.
- Close proof needed: rebuild/restart installed app and run a Gemini call from a VM; `/status` and the toolbar should show non-zero tokens/cost matching the Stats tab.


### RHB-019 - Profile cards advertise unprovisionable profiles

- Status: fixed in repo, needs installed `/profiles` and UI proof after rebuild/reinstall.
- Report: clicking the Coding profile card fails with `profile 'coding' has no installed signed catalog revision; install it before creating a VM`.
- Root cause: `/profiles` and `/profiles/catalog` used loose profile TOML + local asset presence to mark a profile launchable, while provision requires a complete installed signed catalog revision and verified archived payload.
- Fix landed: profile asset status now uses `load_complete_installed_profile_revision`, so profiles without a signed installed revision report `state: "error"` and `usable_for_vm: false` even if their asset files exist. The dashboard has an explicit regression that keeps such cards visible but disables launch and does not leak raw asset paths.
- Proof so far: `cargo test -p capsem-service handle_list_profiles -- --nocapture`, `cargo test -p capsem-service profile_catalog -- --nocapture`, full `cargo test -p capsem-service` (115 lib tests + 217 service-bin tests + doc tests), `cargo check -p capsem-service -p capsem-core`, `pnpm exec vitest run src/lib/__tests__/session-runtime-truth.test.ts src/lib/__tests__/gateway-store.test.ts src/lib/__tests__/profile-catalog-section.test.ts`, `pnpm run check`, and `just build-ui` passed.
- Close proof needed: installed `/profiles` shows `coding` unusable or absent from launch until its signed revision is installed; dashboard refuses to create from it while `everyday-work` remains launchable.

## Current Debug Order

1. Close RHB-002 with tests and compile.
2. Close RHB-003 by splitting installed profile UX from catalog/debug lifecycle.
3. Close RHB-004 so New Session launches from clicked profile cards.
4. Rebuild UI/app and run focused frontend tests.
5. Re-run installed CLI smoke: `capsem status`, `capsem run "echo test"`.
6. Re-run package/install proof when user can provide sudo.
