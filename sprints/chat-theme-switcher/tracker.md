# Sprint: Chat Theme Switcher

## Tasks

- [x] Write sprint plan and tracker.
- [x] Import Preline packaged themes.
- [x] Add `/chat` theme select and dark-mode switch.
- [x] Remove raw palette utilities from generated UI renderer path.
- [x] Add semantic-token regression test.
- [x] Browser-verify table under theme and dark mode.
- [x] Testing gate.
- [x] Changelog.
- [ ] Commit.

## Notes

- The purpose is visual verification of semantic tokens, so the controls must change real Preline theme variables, not local mock colors.
- Browser verification set `data-theme="theme-ocean"` and `.dark` on `<html>` and kept the generated table visible with nine rows.
- Full frontend test run passed: 3 files, 15 tests.

## Coverage Ledger

- Unit/contract: `npm test -- --run test/ui-semantic-tokens.test.ts` passes.
- Functional: `npm run ui:build` and `npm run typecheck` pass.
- Adversarial: raw palette utility scan covers `A2Node.svelte`, `ChatPage.svelte`, and `a2ui.ts`.
- E2E/UI: Chrome DevTools check on `/chat?v=theme-switcher` found theme controls, one generated table, nine rows, and successful Ocean/dark-mode toggle.
- Telemetry: not applicable.
- Performance: not applicable.
