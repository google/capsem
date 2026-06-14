# 1.3 Profile Launcher Assets Sprint

## Purpose

Make the Sessions page honor the profile contract: profile launch choices come
from `/profiles/list`, each choice displays the profile-owned icon/name/
description, and session creation is gated by that profile's asset readiness.

## Scope

- Expose profile icons through the profile summary API.
- Load profile summaries and per-profile asset status in the frontend.
- Render one launch control per profile.
- If a profile's assets are missing/downloading, show download state and a
  download action instead of enabling launch.
- When download completes, refresh that profile asset status so the launch
  button becomes active.
- Pass `profile_id` in VM creation requests.

## Done Means

- No hard-coded "code profile only" launcher on the Sessions page.
- Each visible profile launcher uses route-provided icon/name/description.
- Launch is disabled only for the affected profile while assets are not ready.
- Downloading/missing/error status is visible per profile.
- Focused frontend and Rust tests cover the route contract and UI helpers.

## Verification Matrix

- Unit/contract: service profile summary serialization, frontend API/store tests.
- Functional: `pnpm -C frontend check`, focused frontend tests.
- Adversarial: profile creation requests include `profile_id`; no profile
  launch path bypasses asset readiness.
- E2E/VM: not run in this slice unless requested; full release smoke remains
  the VM gate.
