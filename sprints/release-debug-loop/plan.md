# Release Startup Reliability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` for independent implementation slices or `superpowers:executing-plans` for inline execution. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `capsem uninstall -> just install -> capsem status` the release gate for a coherent installed Capsem runtime.

**Architecture:** Runtime replacement is owned by install/update. Asset readiness is owned by the service. Setup owns config/onboarding and observes service truth. UI, tray, app, gateway, and CLI all render the same health model.

**Tech Stack:** Rust CLI/service/gateway, macOS package scripts and LaunchAgent, `just` install recipes, Docker install harness, Astro/Svelte frontend, Python/pytest policy and install tests.

---

## File Structure Map

Sprint control:

- `sprints/release-debug-loop/startup-info.md` - product contract and open decisions.
- `sprints/release-debug-loop/MASTER.md` - meta-sprint board and release holds.
- `sprints/release-debug-loop/plan.md` - implementation plan and proof matrix.
- `sprints/release-debug-loop/TRACKER.md` - live execution checklist and coverage ledger.
- `sprints/release-debug-loop/BUG_HITLIST.md` - original symptom queue mapped into the new sprint model.

Likely implementation areas:

- `justfile` - local install gate and temporary proof commands.
- `scripts/build-pkg.sh` - package payload construction.
- `scripts/pkg-scripts/preinstall` and `scripts/pkg-scripts/postinstall` - native package lifecycle.
- `crates/capsem/src/main.rs` - CLI dispatch for install/setup/status commands.
- `crates/capsem/src/uninstall.rs` - runtime uninstall and future purge split.
- `crates/capsem/src/service_install.rs` - LaunchAgent/systemd registration and status.
- `crates/capsem/src/setup.rs` - setup orchestration and readiness honesty.
- `crates/capsem-core/src/asset_manager.rs` - manifest, asset identity, and download primitives.
- `crates/capsem-service/src/*` - service startup, asset supervisor, saved VM inventory, status.
- `crates/capsem-gateway/src/status.rs` - gateway health/status projection.
- `crates/capsem-tray/*` and `crates/capsem-app/*` - tray/app startup expectations.
- `frontend/src/lib/stores/*` and `frontend/src/lib/components/onboarding/*` - wizard/dashboard startup states.
- `tests/capsem-install/*` - installed-product and package lifecycle tests.
- `tests/test_release_workflow_policy.py` - static policy tests for release workflow rules.
- `frontend/src/lib/__tests__/*` - UI state and settings/provider tests.

## S0 - Startup Contract And Scope Control

**Purpose:** Stop optimizing the wrong chain. Write the desired contract and make stale narrow-fix notes visibly subordinate to it.

- [x] Define `capsem uninstall` as runtime removal that preserves user-owned durable state.
- [x] Define `capsem purge` as explicit destructive removal.
- [x] Define assets as cache except when referenced by saved VMs.
- [x] Define service-owned autonomous asset supervision with no installer reconcile RPC.
- [x] Define the release gate as `capsem uninstall -> just install -> capsem status`.
- [x] Audit and revert the earlier setup-blocking patch because it conflicts with the desired service-owned asset model.
- [x] Remove premature changelog language for the superseded setup-blocking behavior.

**Proof for S0:**

- Docs agree across `startup-info.md`, `MASTER.md`, `plan.md`, `TRACKER.md`, and `BUG_HITLIST.md`.
- No release hold is expressed as a command that does not exist unless the sprint that creates it is named.

## S1 - Package Install And `capsem status` Health Gate

**Purpose:** Make package install and local install robust enough to be the first gate.

**Current slice:** Closed for the install/status scope. `capsem doctor`
preflights through the same reusable typed status health module as
`capsem status` before it provisions a diagnostic VM. `capsem status --json`
now exposes top-level `state`, grouped `checks`, and stable typed issues across
host binaries, helper versions, service units, setup state, assets, app bundle,
service endpoint, and gateway readiness. The black-box install harness covers
missing service, missing helper binaries, setup-state failures, missing MCP
helper binaries, stale helper binary versions, missing manifests, missing
canonical rootfs assets, runtime uninstall preservation, fixture freshness,
and reinstall over stale helpers. Service-owned asset supervisor states move to
S3, saved-VM asset preservation enforcement moves to S4, UI consumption moves
to S6, and update-over-existing moves to S7.

**Behavior to build:**

- `capsem status` reports whether the installed product is coherent.
- `capsem doctor` calls the reusable `capsem status` health check before it
  provisions a diagnostic VM; doctor is deeper VM proof, not a substitute for
  install/startup readiness.
- `.pkg`, `just install`, and CLI runtime uninstall follow the same contract.
- `capsem uninstall` removes old runtime, stale launch agents, stale app/tray/gateway placement, and temp VM state.
- `capsem uninstall` preserves durable config, credential references, saved VM metadata, logs/audit data required by policy, and saved-VM-referenced assets.
- package install starts the service and proves minimal service liveness.
- package install does not rely on setup to download VM assets.

**Diagnostics `capsem status` must cover:**

- installed binary versions and build hash alignment.
- expected binaries present and executable.
- helper binaries, including service, process, gateway, and tray, report the
  installed runtime version.
- LaunchAgent/systemd unit installed and points at current binary paths.
- service process is reachable.
- gateway status is reachable if gateway is part of installed runtime.
- asset status is one of the explicit service states, never absent/unknown due to missing plumbing.
- setup state is readable and honest.
- app/tray bundle state is current enough to launch.
- on macOS real installed runtimes, `/Applications/Capsem.app` is present and
  reported as `app_bundle_missing` when absent.
- durable state preservation policy is visible in the report.

**Tests to add before implementation:**

- clean install succeeds from no prior runtime.
- reinstall over existing runtime replaces binaries and launch agent paths.
- broken service registration is detected by `capsem status`.
- missing binary is detected by `capsem status`.
- missing manifest and missing canonical rootfs are detected by
  `capsem status`.
- doctor preflight fails with the same blocking status issues before VM
  provisioning.
- stale launch agent is detected and repaired by install or reported clearly.
- postinstall failure leaves an actionable diagnostic.
- simulated install tests build the default local host binaries once and refresh copied binaries when `CAPSEM_BIN_SRC` changes,
  so tests cannot pass against stale installed helpers.
- uninstall preserves durable user state.
- uninstall removes temp VM state.
- purge removes durable user state only after explicit purge confirmation.

**Gate after S1 expands `capsem status`:**

```bash
capsem uninstall
just install
capsem status
```

## S2 - Verification Harness

**Purpose:** Turn install/startup/update claims into repeatable black-box proofs.

**Current slice:** Closed for the verification-harness scope. Failed
`capsem status --json` gates now have a standalone
evidence collector in `scripts/capture-install-status.py`. It runs the installed
binary, writes raw stdout/stderr, preserves typed status JSON when the command
emits it, stores grouped status checks in metadata, records command metadata
and `capsem version`, and snapshots the isolated or real `CAPSEM_HOME` tree
shallowly enough to debug partial installs without dumping large asset caches.
It also captures optional `capsem debug` output, service/gateway run-state
breadcrumbs while redacting `gateway.token`, an explicit install-layout index
for helper binaries, manifest files, setup state, the platform service unit,
and the macOS app bundle path, plus `saved-vm-state.json` with persistent
registry summaries, persistent-session tree evidence, saved-VM asset-reference
fields when present, and env-key-only redaction. `just install` runs the
collector after gateway health and before guest DNS/HTTPS. Dirty fixtures cover
missing service helper, missing tray helper, completed-setup dead service, stale
service-unit evidence, missing app-bundle evidence, missing capsem, malformed
saved-VM registry, and timeouts.

**Behavior to build:**

- A harness that runs the product gate from a clean state and from dirty/stale states.
- Failed gate evidence capture for `capsem status --json` that is stable enough
  for CI logs and local release debugging.
- A saved-VM fixture path that captures referenced asset evidence; preservation
  enforcement belongs to S4.
- A partial-install fixture path that can prove diagnostics catch incoherent installs.
- A UI/gateway/service health capture bundle for failed gates.

**Proof matrix:**

- Unit/contract: uninstall/purge policy, status report schema, status model mapping.
- Functional: CLI status checks against synthetic installed layouts; doctor
  preflight rejects status blockers before booting a diagnostic VM.
- Adversarial: corrupt manifests, bad permissions, stale units, missing app bundle, dead service, partial package failure.
- E2E/install: clean install, reinstall, uninstall/install, update-over-existing.
  The simulated reinstall-after-uninstall and reinstall-over-corrupt-helper
  gates are covered now; true update-over-existing belongs with S7 semantics.
- UI/product: dashboard and wizard render service/asset/setup states from captured status (S6).
- Telemetry/observability: failed install gate captures enough diagnostics to debug without screenshots.

**Current proof:**

- `cargo test -p capsem status::tests -- --nocapture`
- `uv run pytest tests/test_install_status_capture.py tests/capsem-install/test_fixture_refresh.py tests/capsem-install/test_reinstall.py::TestReinstall::test_reinstall_after_runtime_uninstall_restores_status_layout tests/capsem-install/test_reinstall.py::TestReinstall::test_reinstall_over_existing_replaces_corrupt_helper tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_capture_records_partial_install_missing_helper tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_capture_records_missing_tray_helper tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_capture_records_dead_service -q`
- `uv run pytest tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_typed_install_blockers tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_missing_helper_binary tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_missing_mcp_helper_binary tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_stale_process_helper_binary tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_corrupt_setup_state tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_missing_asset_manifest tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_missing_rootfs_asset tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_accepts_completed_setup_state -q`
- `uv run pytest tests/test_release_workflow_policy.py::test_local_install_verifies_fresh_install_and_guest_network -q`

## S3 - Service Asset Supervisor And Consumer Audit

**Purpose:** Move asset truth into the service and make every consumer read it.

**Current slice:** Closed for the service asset supervisor scope. The service now owns a typed asset
state machine (`checking`, `updating`, `ready`, `error`) with progress, retry,
missing-artifact, version, and arch detail. The daemon starts the supervisor at
startup, re-checks on a timer, downloads missing current-version assets in the
background, exposes the same model through service `/list` and `/setup/assets`,
and preserves it through gateway `/status`, `capsem status --json`, frontend
runtime types, and the tray menu. The native Tauri app has no separate asset
status parser; it consumes the frontend gateway model. Local release-fixture
tests prove the spawned background loop downloads missing assets to `ready` and
reports retryable `error` when the release source fails.

**Behavior to build:**

- Service computes desired assets from installed runtime version and saved VM references.
- Service checks assets on start, on a timer, and after version/install metadata changes.
- Service downloads missing required assets in the background.
- Service status exposes `checking`, `updating`, `ready`, and `error` with progress and retry detail.
- No installer-only asset reconcile command is needed.
- Tray, app, gateway, CLI, wizard, and dashboard consume the same status model.

**Tests to add before implementation:**

- [x] service starts with missing assets and reports `updating`.
- [x] service reports byte/artifact progress while downloading.
- [x] service reports retryable error when release source is unreachable.
- [x] service reports ready after verified assets land.
- [x] gateway and CLI preserve the full service asset state.
- [x] frontend runtime consumer does not collapse `updating` into unknown.
- [x] tray/app consumers do not collapse `updating` into unknown.
- [x] missing release assets move from `updating` to `ready` after the supervisor downloads them.
- [x] unreachable release source becomes retryable asset `error` through the real supervisor loop.

**Current proof:**

- `cargo test -p capsem-service asset_supervisor -- --nocapture`
- `cargo test -p capsem-service handle_list_exposes_service_asset_supervisor_state -- --nocapture`
- `cargo test -p capsem-gateway fetch_status_preserves_service_asset_state -- --nocapture`
- `cargo test -p capsem status::tests -- --nocapture`
- `cargo test -p capsem-tray spec_preserves_asset_updating_state_and_disables_new_session -- --nocapture`
- `npx vitest run src/lib/__tests__/session-runtime-truth.test.ts`

## S4 - Saved VM Asset Dependencies

**Purpose:** Stop treating rootfs as disposable when a saved VM depends on it.

**Status:** Implemented for local/service status scope on 2026-05-14. Live
update-over-existing proof remains in S7 and the final meta-sprint gate.

**Behavior to build:**

- Saved VM metadata records base rootfs digest/version, kernel digest/version, initrd digest/version, architecture, and guest ABI compatibility.
- Asset cleanup preserves blobs referenced by saved VMs.
- Service status reports missing saved-VM dependencies separately from missing current-version assets.
- VM launch refuses unsafe startup with a clear recovery path when a saved VM base is missing.

**Tests to add before implementation:**

- saved VM prevents referenced rootfs deletion.
- temp VM does not prevent asset cleanup.
- update preserves old rootfs when a saved VM references it.
- missing saved-VM rootfs is reported distinctly from current rootfs updating.
- launch of saved VM with missing base asset fails with actionable status.

**Verification added:**

- `cleanup_unused_assets_preserving` keeps saved-VM-referenced hash-named files
  in flat and arch-specific asset layouts while deleting unreferenced temp
  assets.
- persistent-registry round-trip preserves base asset identity and still loads
  legacy records without it.
- service `/list` reports saved-VM dependency gaps separately from current
  asset readiness.
- saved-VM resume fails before launch when the pinned rootfs is missing.
- fork from running and stopped persistent VMs inherits base asset identity.
- gateway, tray, frontend type, and CLI status tests preserve and surface
  saved-VM dependency gaps, including typed `saved_vm_asset_missing` status
  blockers.

## S5 - `capsem-setup` Hardening

**Purpose:** Make setup a config/onboarding workflow that is honest about readiness.

**Current slice (2026-05-14):** Closed for the setup-hardening scope. Setup
summary now gates on live service `/list` asset truth, keeps config completion
non-blocking while reporting pending readiness for
unavailable/checking/updating/error service states (`vm_verified=false`), fails
on unknown/inconsistent service truth, and only marks VM readiness complete
when service reports `ready`. Packaging-safe harness proofs now cover setup
rerun idempotence/provider fallback and explicit pending-readiness output when
service never becomes live.

**Behavior to build:**

- Setup requires service liveness for service-backed readiness.
- Setup fans out independent work where safe: provider detection, repo detection, corp config refresh, security preset checks.
- Setup observes service asset status rather than owning asset download.
- Setup is idempotent and safe to rerun after reinstall/update.
- Setup does not claim VM readiness when service truth is unavailable, non-ready, or inconsistent.
- Provider/settings parsing failures produce actionable fallback UI and diagnostics.

**Tests to add before implementation:**

- setup surfaces explicit pending readiness when service is not live.
- setup completes config work while assets are `updating`.
- setup does not claim VM readiness while assets are still updating.
- setup rerun after reinstall preserves accepted provider/security choices.
- provider settings empty/malformed tree still renders provider rows or clear error.

## S6 - UI Wizard/Dashboard Startup States

**Purpose:** Make UI startup states truthful and recoverable.

**Current slice (2026-05-14):** Closed for the UI startup-state scope.
Dashboard session creation now requires both service-running and assets-ready
truth, surfaces explicit service-offline/asset-state/saved-VM-dependency
messaging, keeps refresh status available while blocked, and exposes retry
setup affordances plus inline retry errors on retryable service asset errors.
Onboarding welcome/ready views consume and render the same service/asset truth
states.

**Behavior to build:**

- Wizard/dashboard show service offline, service starting, checking, updating, ready, error, and saved-VM dependency missing.
- Create/run actions are disabled with explicit reasons until prerequisites are ready.
- Retry is available when service marks an asset/setup error retryable.
- UI does not silently return from failed setup or status fetch.
- Tray/app entry points match dashboard status.

**Tests to add before implementation:**

- wizard renders `updating` with progress.
- dashboard renders missing saved-VM dependency.
- retry button calls the correct existing recovery endpoint or launches the designed recovery flow.
- service offline state does not look like an empty VM list.
- provider onboarding remains usable when settings fetch fails.

## S7 - Update/Uninstall/Purge Integration

**Purpose:** Connect all contracts into the release path.

**Behavior to build:**

- Update verifies new payload before runtime uninstall.
- Update runs runtime uninstall, then fresh install.
- Update never runs purge.
- Purge is explicit and destructive.
- Package update, local `just install`, and future updater flow share the same health gate.
- Old assets are pruned only after saved VM references are checked.

**Tests to add before implementation:**

- update over existing runtime performs uninstall/install and preserves durable state.
- update does not delete saved-VM-referenced rootfs.
- purge deletes durable state after explicit confirmation.
- failed payload verification does not uninstall current runtime.
- failed fresh install produces `capsem status` evidence.

## Meta-Sprint Done Definition

- `capsem uninstall -> just install -> capsem status` passes.
- A saved VM fixture survives update when it references an older rootfs.
- A temp VM fixture does not preserve disposable assets.
- Service asset status is observable from CLI, gateway, UI, tray/app surfaces.
- Setup can run while assets update in the background without lying about VM readiness.
- UI/wizard/dashboard show recoverable progress/error states.
- Package install, local install, and update share the same runtime replacement contract.
- Coverage ledger names any missing unit, functional, adversarial, E2E, UI, telemetry, or performance proof.
