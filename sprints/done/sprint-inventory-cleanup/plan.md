# Sprint Inventory Cleanup Plan

## Goal

Preserve the Profile V2 rescue work on `main`, make the active sprint authority
obvious, and move obsolete planning boards out of the top-level sprint list so
future work does not restart from stale plans.

## Decisions

- `origin/main` already points at the rescued Profile V2 release commit
  `6daf264a`; no history rewrite or cherry-pick is needed.
- Historical sprint folders are moved to `sprints/retired/`, not deleted.
- `sprints/policy-settings-profiles/` remains the active Profile V2 authority.
- The next release-blocking Profile V2 work is tracked in the numbered
  `Sxx-*` files under `sprints/policy-settings-profiles/`, especially S08b,
  S09, S11, S15, S16, S18, and S19.

## Files

- Add `sprints/README.md` as the sprint inventory entry point.
- Update `sprints/policy-settings-profiles/RETIRED-LEGACY-SPRINTS.md`.
- Move obsolete top-level sprint folders under `sprints/retired/`.
- Keep this cleanup plan and tracker as the audit trail for the move.

## Done

- The top-level `sprints/` directory shows active/current work first.
- Retired historical folders are still searchable under `sprints/retired/`.
- The inventory names the active Profile V2 board and the next release holds.
- Verification proves there are no broken tracked sprint docs caused by the
  move.

## Testing Matrix

- Unit/contract: not applicable; documentation-only cleanup.
- Functional: `find`/`rg` inventory checks confirm moved directories and active
  authority docs exist.
- Adversarial: check for top-level copies of retired sprint names.
- E2E/VM: not applicable.
- Telemetry: not applicable.
- Performance: not applicable.
