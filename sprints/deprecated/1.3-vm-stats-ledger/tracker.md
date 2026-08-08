# Sprint: 1.3 VM Stats Ledger

## Tasks

- [x] Plan and scope recorded.
- [x] Add typed frontend VM ledger API helpers.
- [x] Refactor Stats tab onto current session DB tables and ledger routes.
- [x] Add Security/DNS/Process/Substitution coverage to the Stats tab.
- [x] Fix raw inspector columnar response rendering/sorting.
- [x] Update inspector preset SQL.
- [x] Update changelog.
- [x] Run frontend verification.
- [ ] Commit and push.

## Notes

- Current session DB truth is typed event tables plus `security_rule_events`.
  This sprint does not invent a second stats store.
- The Stats tab now reads fixed SQL against typed session tables and VM-scoped
  `/security`, `/detection`, and `/enforcement` latest/status routes. The
  detail drawer renders the full stored row, plus rule/event JSON for ledger
  rows.

## Coverage Ledger

- Unit/contract: `pnpm -C frontend test src/lib/__tests__/api.test.ts`
  (`66 passed`) covers VM ledger helper routes and columnar inspect response
  shape.
- Functional: `pnpm -C frontend check`, `pnpm -C frontend build`.
- Adversarial: inspector still validates SELECT-only SQL; Stats tab uses fixed
  bounded SQL strings and typed route helpers rather than user-authored query
  strings.
- E2E/UI: Not run against a live VM in this slice; final release smoke must
  click through a real VM with populated session.db.
- Telemetry: Not changed; this only reads existing session DB and ledger routes.
- Performance: Stats list queries are capped at 100-200 rows and aggregate in
  SQLite.
- Missing/deferred: Service-side typed stats DTOs would be cleaner than SQL for
  the whole tab, but current inspect route plus ledger routes are the existing
  contract for raw DB inspection.
