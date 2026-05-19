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

Gaps:

- `capsem update --assets` still uses the older release asset manager path and
  verified asset manifest, not the Profile V2 catalog/profile asset supervisor.
- There is no manual service endpoint to force a profile asset check/download
  through the same supervisor path that background reconciliation uses.
- Background download events are mostly state transitions; they lack a stable
  structured event trail for check start, missing assets, download start,
  progress, verify success, install success, retryable failure, and terminal
  failure.
- Status shows the latest health snapshot but not enough provenance for profile
  id/revision, selected catalog payload hash, asset URL/hash, or last checked
  time.
- Tests cover background reconciliation and status preservation, but not
  `capsem update --assets -> service trigger -> status progress -> logs`.

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

1. **Manual profile asset reconcile endpoint**
   - Add a service route such as `POST /setup/assets/reconcile`.
   - It calls the existing `AssetSupervisor::ensure_assets_once()` path.
   - It returns the final `AssetHealth` snapshot and a typed result:
     `already_ready`, `downloaded`, `checking`, `error`.
   - It must be idempotent under concurrent calls.

2. **CLI update integration**
   - Change `capsem update --assets` to call the service endpoint when the
     service is running.
   - Print the same operator language as status: profile asset check started,
     already ready, downloaded/refreshed, still updating, or failed.
   - Remove old asset-manifest authority from this command path. If an offline
     fallback remains for development, gate it behind an explicit dev-only
     message and test.

3. **Structured asset lifecycle logging**
   - Emit structured service logs for:
     `profile_asset_check_start`, `profile_asset_check_ready`,
     `profile_asset_missing`, `profile_asset_download_start`,
     `profile_asset_download_progress`, `profile_asset_verify_ok`,
     `profile_asset_install_ok`, `profile_asset_download_retryable_error`, and
     `profile_asset_check_error`.
   - Include profile id, revision, arch, logical asset name, expected hash,
     target path, URL host/path, byte counts, retry count, and elapsed time.
   - Avoid logging secrets or signed URL credentials.

4. **Status/debug provenance**
   - Extend `AssetHealth` or a sibling status payload with profile id/revision,
     payload hash when known, last check time, and last transition/event.
   - Make `capsem status` render these details compactly.
   - Add debug-report asset provenance for profile asset source and latest
     supervisor state.

5. **Concurrency and cleanup hardening**
   - Add per-asset or profile-level locks beyond the current in-process run
     lock where needed.
   - Prove two manual triggers do not duplicate downloads.
   - Prove cleanup refuses while a download is in progress and cannot delete a
     temp/target asset being installed.

## Testing Matrix

- Unit/contract:
  - `AssetSupervisor` emits expected health/provenance transitions.
  - Manual endpoint maps supervisor outcomes into stable JSON.
  - CLI parser/output covers `capsem update --assets` service success/failure.
- Functional:
  - Fake asset server + service endpoint downloads missing profile assets and
    returns final health.
  - `capsem update --assets` calls the service endpoint and `capsem status`
    reflects progress/readiness.
- Adversarial:
  - Hash mismatch deletes temp file and logs terminal failure.
  - 404/503 records retryable failure with retry count.
  - Duplicate manual triggers share the same reconcile run.
  - Cleanup during update returns conflict.
- E2E/VM or integration:
  - Create VM with missing profile assets triggers download, then boots with
    pinned hashes.
- Telemetry/observability:
  - Service logs contain lifecycle event names and fields for a successful
    download and a retryable failure.
  - Debug report includes the latest profile asset state.
- Performance:
  - Repeated checks when assets are present do not hash every large file on hot
    `/list` or status paths.
  - Concurrent triggers do not create duplicate network downloads for the same
    asset hash.

## Done

- `capsem update --assets` is a Profile V2 service-triggered asset reconcile.
- `capsem status` clearly reports profile asset check/update/readiness.
- Background and manual checks use one code path.
- Logs and debug report explain checks/downloads without sensitive data.
- Old asset manifest authority is gone from user-facing update flows.
- Focused tests plus package gates are recorded in `tracker.md`.
