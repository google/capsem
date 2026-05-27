# Sprint: Sprint Inventory Cleanup

## Tasks

- [x] Confirm rescued Profile V2 release commit is on `origin/main`.
- [x] Identify retired planning boards already superseded by
  `policy-settings-profiles`.
- [x] Move retired boards under `sprints/retired/` without deleting history.
- [x] Add top-level sprint inventory.
- [x] Update retired sprint guardrail paths.
- [x] Run documentation inventory checks.
- [x] Commit and push cleanup.

## Notes

- `HEAD`, `origin/main`, and tag `v1.2.1779673506` all pointed at
  `6daf264a` before cleanup began.
- The existing `policy-settings-profiles/RETIRED-LEGACY-SPRINTS.md` already
  retired several legacy folders as planning authority; this cleanup makes that
  retirement visible in the filesystem.

## Coverage Ledger

- Unit/contract: not applicable; no runtime code changed.
- Functional: `find sprints -maxdepth 1 -type d -print | sort` confirms the
  active top-level sprint inventory and retired archive split.
- Adversarial: retired-name checks confirm the obsolete directories are absent
  from top-level `sprints/` and present under `sprints/retired/`.
- E2E/VM: not applicable.
- Telemetry: not applicable.
- Performance: not applicable.
- Missing/deferred: broader content deduplication inside retired historical
  boards is intentionally deferred; this sprint only relocates and indexes.

## Verification

- `rg` found no non-retired, non-done references to the retired top-level
  sprint paths.
- `git diff --check` passed.
