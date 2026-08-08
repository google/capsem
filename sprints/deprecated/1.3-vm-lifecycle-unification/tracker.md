# Sprint: 1.3 VM Lifecycle Unification

## Tasks

- [x] Plan and scope recorded.
- [x] Unify Sessions VM table and row actions.
- [x] Make profile launcher create named retained VMs.
- [x] Make custom session dialog require a name and create retained VMs.
- [x] Align active VM toolbar lifecycle actions.
- [x] Burn frontend user-visible tmp/ephemeral/persistent wording.
- [x] Update changelog.
- [x] Run frontend and grep verification.
- [ ] Commit and push.

## Notes

- Backend API structs still include `persistent` because service resume/fork/save
  internals use that storage contract today. This sprint removes the user-facing
  split and save/persist UI, not the service storage implementation.
- Browser smoke on `http://127.0.0.1:5173/` loaded the app while the service was
  offline. The offline overlay contains no old VM-class wording; route-backed VM
  rows need a running service for manual click verification.

## Coverage Ledger

- Unit/contract: `pnpm -C frontend test src/lib/__tests__/api.test.ts`
  (`63 passed`) after deleting the stale `persistVm` client test.
- Functional: `pnpm -C frontend check`, `pnpm -C frontend build`.
- Adversarial: `rg` guard over `frontend/src` found no user-facing
  `ephemeral`, `temporary session`, `Persistent`, `Save Session`,
  `Destroy Session`, `persistVm`, or `vmStore.persist` references in the edited
  UI/client surfaces.
- E2E/UI: Browser loaded the dev server; full VM action click-through requires a
  running gateway/service and belongs in the final release smoke.
- Telemetry: Not touched.
- Performance: Not touched.
- Missing/deferred: CLI/backend still carry internal `persistent` storage fields,
  `/vms/{id}/save`, and old command text; burn that in the runtime/API cleanup
  sprint rather than changing service semantics under a frontend slice.
