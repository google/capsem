# S07c - Profile Asset Update Orchestration

## Goal

Make profile-owned asset checks, background downloads, manual update commands,
status output, and logs behave as one production operator workflow.

S07a proved the core pieces: the service has an `AssetSupervisor`, profile VM
asset declarations can download into the service asset directory, `/setup/assets`
and `/list` expose readiness/progress, `capsem status` preserves the service
asset state, and cleanup refuses to run while assets are checking or updating.
That is not enough. Operators need an explicit update/check path that uses the
same Profile V2 source of truth, explains what it is doing, and leaves an audit
trail.

## Current State

Landed:

- `AssetSupervisor` runs in the background on service startup and periodically
  calls `ensure_assets_once()`.
- Profile-backed downloads stream to a temp file, hash as bytes arrive, rename
  only after the expected BLAKE3 hash matches, and publish per-file progress in
  the in-memory asset health snapshot.
- `/setup/assets`, `/list`, and `capsem status` surface `checking`, `updating`,
  `ready`, `error`, missing assets, progress, retry count, and retryability.
- `POST /setup/assets/cleanup` refuses cleanup unless the supervisor reports
  `ready`.
- `POST /setup/assets/reconcile` now forces the same service-owned
  `AssetSupervisor::ensure_assets_once()` path and returns `already_ready`,
  `downloaded`, `checking`, or `error` with final health.
- `capsem update --assets` now calls `/setup/assets/reconcile`; it no longer
  reads old asset-only manifests or downloads VM assets outside the service.
- Service asset health timestamps are preserved through `capsem status --json`
  and rendered in text status output.
- Service/CLI asset health now carries explicit Profile V2 provenance:
  `profile_id`, `profile_revision`, installed profile payload hash, redacted
  per-asset source URL/hash/size/content-type, asset version, arch, and check
  timestamp.
- Structured lifecycle logs now cover check start/ready/error, missing assets,
  download start/progress, hash verification, install success, and retryable
  download failure. Download URLs are logged without credentials or query
  strings.
- First-use VM create/run now awaits the same Profile V2 asset reconciler
  before process spawn, so missing selected-profile assets download through the
  service path instead of failing on a stale initial health snapshot.
- Create-from-source, fork, and persist derive base asset identity from the
  VM's profile pin and reject conflicts with the duplicate registry side field.

Gaps:

- Tests cover the service trigger, CLI summary rendering, background
  reconciliation, status timestamp/provenance preservation, debug provenance,
  first-use VM create reconciliation, profile-pin asset authority, concurrent
  manual reconcile locking, active cleanup refusal, and log URL redaction, but
  not full `capsem update --assets -> live service -> status progress -> logs`
  E2E or a live hypervisor boot against freshly downloaded assets.

## Product Contract

- Profile assets are updated from the signed Profile V2 catalog/profile payload,
  never from the old asset-only manifest.
- `capsem update --assets` talks to the running service when available and
  triggers a Profile V2 asset reconcile. In development/offline fallback, it
  must say exactly why the service path was unavailable; it must not silently
  use old manifest authority.
- Background and manual triggers use the same per-asset locking, temp-file,
  streaming hash, and rename path.
- `capsem status` and `capsem status --json` report enough detail for support:
  profile id, revision, payload hash when known, arch, missing assets, current
  file progress, retry count, retryable flag, last check time, and last error.
- Logs have stable event names/fields for asset check and download lifecycle.
  The operator story should be reconstructable from service logs without
  reading in-memory state at the exact right moment.
- Cleanup and VM creation must coordinate with in-progress asset work. No
  cleanup of assets being downloaded; no duplicate network download for the
  same hash when two callers trigger reconciliation.

## Implementation Slices

1. [x] **Manual profile asset reconcile endpoint**
   - Add a service route such as `POST /setup/assets/reconcile`.
   - It calls the existing `AssetSupervisor::ensure_assets_once()` path.
   - It returns the final `AssetHealth` snapshot and a typed result:
     `already_ready`, `downloaded`, `checking`, `error`.
   - It must be idempotent under concurrent calls.

2. [x] **CLI update integration**
   - Change `capsem update --assets` to call the service endpoint when the
     service is running.
   - Print the same operator language as status: profile asset check started,
     already ready, downloaded/refreshed, still updating, or failed.
   - Remove old asset-manifest authority from this command path. If an offline
     fallback remains for development, gate it behind an explicit dev-only
     message and test.

3. [~] **Structured asset lifecycle logging**
   - Emit structured service logs for:
     `profile_asset_check_start`, `profile_asset_check_ready`,
     `profile_asset_missing`, `profile_asset_download_start`,
     `profile_asset_download_progress`, `profile_asset_verify_ok`,
     `profile_asset_install_ok`, `profile_asset_download_retryable_error`, and
     `profile_asset_check_error`.
   - Include profile id, revision, arch, logical asset name, expected hash,
     target path, URL host/path, byte counts, retry count, and elapsed time.
   - Avoid logging secrets or signed URL credentials.

4. [~] **Status/debug provenance**
   - Extend `AssetHealth` or a sibling status payload with profile id/revision,
     payload hash when known, last check time, and last transition/event.
   - Make `capsem status` render these details compactly.
   - Add debug-report asset provenance for profile asset source and latest
     supervisor state.
   - Landed: `profile_id`, `profile_revision`, installed profile payload hash,
     and redacted per-asset source/hash metadata are explicit fields in
     service/CLI asset health, setup asset status, reconcile/list payloads,
     text status, and debug reports.

5. [x] **Concurrency and cleanup hardening**
   - Add per-asset or profile-level locks beyond the current in-process run
     lock where needed.
   - Prove two manual triggers do not duplicate downloads.
   - Prove cleanup refuses while a download is in progress and cannot delete a
     temp/target asset being installed.

6. [x] **First-use VM create integration**
   - `handle_provision` / `handle_run` await `AssetSupervisor::ensure_assets_once()`
     before process spawn when creating from the selected profile.
   - Create-from-source, fork, and persist use the source VM's profile pin as
     the boot-asset authority and fail closed on pin/registry drift.
   - Focused service tests prove missing profile assets download before spawn
     and pin-side asset authority is preserved.

## Testing Matrix

- Unit/contract:
  - `AssetSupervisor` emits expected health/provenance transitions.
  - Manual endpoint maps supervisor outcomes into stable JSON.
  - CLI parser/output covers `capsem update --assets` service success/failure.
  - Landed: `cargo test -p capsem-service asset_supervisor --lib` covers
    background download, retryable errors, progress state, and log URL
    redaction. `cargo test -p capsem profile_asset_reconcile_summary_line`
    covers CLI output summaries.
  - Landed: `startup_asset_requirement_includes_installed_profile_payload_provenance`
    proves startup asset health includes installed revision/payload provenance,
    and `profile_asset_provenance_redacts_source_urls` proves signed URL
    credentials/query strings stay out of status/debug provenance.
- Functional:
  - Fake asset server + service endpoint downloads missing profile assets and
    returns final health.
  - `capsem update --assets` calls the service endpoint and `capsem status`
    reflects progress/readiness.
  - Landed: `cargo test -p capsem-service handle_asset_reconcile` covers the
    service endpoint downloading missing profile assets and the already-ready
    outcome plus profile id/revision in the returned health. `cargo test -p
    capsem parse_update_assets` keeps CLI parsing wired.
  - Landed: `provision_attempt_reconciles_profile_assets_on_first_use_create`
    proves the create path downloads missing selected-profile assets via the
    service reconciler before process spawn.
- Adversarial:
  - Hash mismatch deletes temp file and logs terminal failure.
  - 404/503 records retryable failure with retry count.
  - Landed: `handle_asset_reconcile_concurrent_calls_share_one_download_run`
    proves two concurrent manual triggers issue exactly one GET per required
    profile asset.
  - Landed: `handle_asset_cleanup_refuses_during_active_profile_download`
    proves cleanup returns conflict while a profile asset download is active.
- E2E/VM or integration:
  - Create VM with missing profile assets triggers download, then boots with
    pinned hashes.
  - Landed integration proof: focused service create-path coverage downloads
    missing profile assets before spawn; existing process-spawn wiring passes
    expected kernel/initrd/rootfs hashes to `capsem-process`. Remaining live VM
    proof is a real hypervisor boot with freshly downloaded assets.
- Telemetry/observability:
  - Service logs contain lifecycle event names and fields for a successful
    download and a retryable failure.
  - Debug report includes the latest profile asset state.
  - Landed: structured log calls are in the service asset supervisor for the
    lifecycle event names; focused URL-redaction coverage prevents signed URL
    query/credential leakage.
  - Landed: service debug reports now serialize Profile V2 asset health instead
    of legacy asset manifest presence/hash fields.
  - Landed: debug reports include explicit profile id/revision from asset
    health plus payload hash and per-asset source/hash metadata.
- Performance:
  - Repeated checks when assets are present do not hash every large file on hot
    `/list` or status paths.
  - Concurrent triggers do not create duplicate network downloads for the same
    asset hash.

## Done

- `capsem update --assets` is a Profile V2 service-triggered asset reconcile.
- `capsem status` clearly reports profile asset check/update/readiness.
- `capsem status --json` and debug reports include profile payload hash plus
  redacted per-asset source/hash metadata.
- Background and manual checks use one code path.
- Logs and debug report explain checks/downloads without sensitive data.
- Concurrent manual reconcile requests share the supervisor run lock and do not
  duplicate network downloads.
- Cleanup refuses active downloads and leaves stale files untouched until asset
  health is ready.
- First-use VM create/run triggers Profile V2 reconciliation before spawning,
  and source/fork/persist derive boot-asset identity from the profile pin.
- Old asset manifest authority is gone from user-facing update flows.
- Old Rust `ManifestV2` parsing, verified loading, direct downloading, and
  manifest-driven cleanup paths are removed from the runtime crates.
- Focused tests plus package gates are recorded in `tracker.md`.
