# Release Startup Reliability Master

Last updated: 2026-05-14

## Mission

Make Capsem startup/install reliable enough that the release gate is a real
installed-product proof, not a pile of optimistic local checks.

The primary gate for this meta-sprint is:

```bash
capsem uninstall
just install
capsem status
```

S1 expands `capsem status` into the product oracle. Until then, the temporary
proof set is explicit service, gateway, asset, setup, UI/tray, and saved-VM
checks documented in `plan.md`.

## Product Contract

The startup contract lives in `startup-info.md`. It is the source of truth for:

- `capsem uninstall` as runtime removal, not user-data deletion.
- `capsem purge` as the destructive reset path.
- update as verified payload plus runtime uninstall plus fresh install.
- service-owned asset supervision without an installer reconciliation RPC.
- saved VM dependencies on rootfs/kernel/initrd asset identities.
- setup as config/onboarding orchestration, not asset ownership.
- UI, wizard, dashboard, tray, app, gateway, and CLI reading one status truth.

## Sprint Board

| Sprint | Status | Purpose | Release Hold It Removes |
| --- | --- | --- | --- |
| S0 - Startup Contract And Scope Control | Done | Write the product contract and subordinate the old bug hitlist to it. | Prevents more narrow patches against the wrong ownership model. |
| S1 - Package Install And `capsem status` Health Gate | Done | Harden `.pkg`, `just install`, runtime uninstall, service registration, and expand status into the diagnostic oracle. | Installs can no longer silently produce incoherent runtime state. |
| S2 - Verification Harness | Done | Build repeatable black-box install/startup/update proofs around the real product path. | Release gate stops depending on manual screenshots and ad hoc commands. |
| S3 - Service Asset Supervisor And Consumer Audit | Done | Make the service autonomously supervise assets on start, timer, and version change; audit tray/app/gateway/CLI consumers. | Assets stop being a setup-side hidden prerequisite. |
| S4 - Saved VM Asset Dependencies | Done | Persist and honor saved VM base asset identities; protect referenced blobs. | Updates/uninstalls cannot strand saved VMs by deleting their rootfs lineage. |
| S5 - `capsem-setup` Hardening | In Progress | Make setup idempotent, correctly launched, fan-out capable, and status-aware. | Setup stops claiming readiness it does not own or prove. |
| S6 - UI Wizard/Dashboard Startup States | Not Started | Show service, asset, saved-VM, setup, and retry states in wizard/dashboard. | UI stops returning silently or hiding blocked startup work. |
| S7 - Update/Uninstall/Purge Integration | Not Started | Tie package update, uninstall, purge, setup, service, and UI contracts together. | Update becomes an end-to-end runtime replacement path with durable-state safety. |

## Symptom Mapping

The old B1/B2/B3 bugs remain real evidence, but they are no longer the sprint
structure.

| Original Bug | New Home | Notes |
| --- | --- | --- |
| B1 - VM assets missing / setup chain broken | S1, S3, S5, S6 | Earlier setup-blocking patch is a superseded experiment; final fix is service-owned asset truth plus honest setup/UI behavior. |
| B2 - AI provider onboarding/settings parsing broken | S5, S6 | Provider settings parsing belongs to setup hardening and wizard resilience. |
| B3 - VM list/session UI broken after launch | S2, S3, S4, S6 | Must be reproduced after install/service/assets are made observable. |

## Release Holds

- Do not mark this meta-sprint complete until `capsem uninstall -> just install -> capsem status` passes on an installed product.
- Do not accept a fix that requires the installer to call a one-off asset reconciliation RPC.
- Do not delete assets referenced by saved VMs in uninstall/update/purge-adjacent cleanup.
- Do not let setup mark the machine ready when service status is unavailable, unknown, or still updating.
- Do not let UI/wizard/dashboard silently return from asset/setup failures.
- Do not treat package install, `just install`, and direct CLI setup as equivalent until S1/S2 prove the contracts.

## Current Active Work

S5 remains active for a final proof pass. The current S5 hardening slice is in
place: setup summary now polls service `/list` truth, leaves `vm_verified=false`
for unavailable/checking/updating/error asset states, only sets `vm_verified`
when service assets are truly `ready`, and no longer blocks setup on
setup-owned asset downloads. The full local gate (`just test`) is green,
including install E2E.

S0 is in review. S0 output:

- `startup-info.md`
- this `MASTER.md`
- `plan.md`
- `TRACKER.md`
- updated `BUG_HITLIST.md` that maps old bugs into the new meta-sprint

S1 is closed for the current install/status scope: `capsem doctor` preflights
through the same typed status health module before provisioning a diagnostic VM,
and `capsem status --json` now exposes `state`, grouped `checks`, and stable
issue reports for host binaries, helper versions, service units, setup state,
assets, app bundle, service endpoint, and gateway readiness. Black-box install
harness coverage exercises missing service/MCP helpers, stale
service/process/gateway/tray helper versions, setup-state blockers, missing
manifests, missing canonical boot assets, runtime uninstall preservation,
fixture freshness, and reinstall over stale helpers.

S2 is closed for the verification harness scope: `scripts/capture-install-status.py`
captures installed `capsem status --json` evidence into a deterministic bundle
with raw stdout/stderr, parsed status JSON, grouped status checks in metadata,
command metadata, version/debug output, redacted run-state breadcrumbs,
install-layout evidence, macOS app-bundle evidence, saved-VM registry/session
summaries, and saved-VM asset-reference fields when present. `just install`
runs this capture after gateway health and before guest DNS/HTTPS. Dirty
fixtures cover partial installs, missing tray helper, dead service, stale
service units, malformed saved-VM registry, missing app bundle, timeouts, and
missing `capsem`.

S3 is closed for the service asset supervisor scope: a service-owned asset state
machine starts in `checking`, moves to `updating` while required current-version
assets are missing, downloads in the background from the release source, reports
progress/retry detail, and exposes the same typed asset state through service
`/list`, gateway `/status`, CLI status consumers, frontend types, and the tray
menu. The native Tauri app has no separate asset status parser. Local
release-fixture tests prove the spawned background loop can download missing
assets to `ready` and report retryable `error` when the release source fails.

S4 is closed for the saved-VM asset dependency scope: persistent VM records can
store the base asset identity they depend on, forks inherit that identity, asset
cleanup preserves referenced hash-named files, saved-VM resume/clone resolves
pinned assets instead of current assets, and missing saved-VM dependencies are
reported separately through service/gateway/tray/frontend/CLI status. The live
update-over-existing proof remains in S7 and the final meta-sprint gate.
