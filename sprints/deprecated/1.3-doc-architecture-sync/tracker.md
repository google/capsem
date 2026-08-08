# Sprint: 1.3 Documentation Architecture Sync

## Tasks

- [x] Audit public docs and skills for stale 1.2/pre-rescue architecture language.
- [x] Patch service/API docs to current profile-scoped route contract.
- [x] Patch security policy/plugin docs to current single-rail rule/plugin model.
- [x] Patch session telemetry/stats docs to current DB tables, Stats tab, and Inspector behavior.
- [x] Patch profile/assets/settings docs and skills to current ownership model.
- [x] Patch CLI/MCP/doctor docs and skills where they still teach temporary/persistent/setup-era flows.
- [x] Add changelog docs note.
- [x] Run documentation verification.
- [ ] Commit and push.

## Notes

- Historical release notes can remain historical.
- No compatibility/fallback language should be added. Docs should describe the strict route/config contract.

## Coverage Ledger

- Docs grep guard: `rg` guard over current docs/skills for retired setup, Policy V2, old route, old table, and user.toml strings. Only the intentional negative phrase `not settings-owned AI provider toggles` remains.
- Docs build: `pnpm -C docs build` passed.
- Missing/deferred: Historical release notes/changelog history were not rewritten.
