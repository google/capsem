# Sprint: 1.3 Profile Launcher Assets

## Tasks

- [x] Expose `icon_svg` in profile summaries.
- [x] Extend frontend profile/provision types with profile identity.
- [x] Render profile launch controls from `/profiles/list`.
- [x] Load and refresh per-profile asset status.
- [x] Ensure download action refreshes and enables launch when ready.
- [x] Pass selected `profile_id` to VM creation.
- [x] Update tests and changelog.
- [x] Run focused verification.
- [ ] Commit and push.

## Notes

- Initial finding: Sessions page still uses a single default-profile
  `vmStore.assetHealth` and creates sessions without a profile id.
- Initial finding: backend profile summary has name/description but does not
  expose `icon_svg`, so the UI cannot reflect profile-owned icon truth yet.
- Implementation: Sessions page now shows one launch button per web-available
  profile. Missing/downloading assets show a download action; ready assets show
  a start action.
- Implementation: custom session dialog now selects a profile from
  `/profiles/list` and passes the selected `profile_id`.
- Implementation: `vmStore.provision()` rechecks selected profile assets before
  calling `/vms/create`.

## Coverage Ledger

- Unit/contract:
  - `cargo test -p capsem-service handle_profiles_list_returns_code_profile_inventory -- --nocapture`
  - `pnpm -C frontend test src/lib/__tests__/api.test.ts`
- Functional:
  - `pnpm -C frontend check`
  - `pnpm -C frontend build`
  - In-app browser navigated to `http://127.0.0.1:5173/`; automated browser
    screenshot was skipped because Playwright/Puppeteer are not installed in
    the frontend workspace.
- Adversarial:
  - `rg` verified all frontend create/run calls include explicit `profile_id`.
- E2E/VM: not run unless runtime boot is touched.
- Telemetry/performance: not applicable.
- Missing/deferred:
  - No VM boot in this slice.
