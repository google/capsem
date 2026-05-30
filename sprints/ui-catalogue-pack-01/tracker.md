# Sprint: UI Catalogue Pack 01

## Tasks

- [x] Write sprint plan and tracker.
- [x] Add typed Rust contracts and lowering for all five blocks.
- [x] Add Preline/Svelte renderers for all five blocks.
- [x] Add Rust/Hyper tests.
- [x] Add frontend semantic and browser verification.
- [x] Testing gate.
- [x] Changelog.
- [x] Commit.
- [x] Follow-up: make `ui.ask` modal behave like a real dialog in Svelte.

## Notes

- User correction: card image support and table filtering/search/pagination are part of Pack 01, not later work.
- `/chat` now renders all authored surfaces so the pack can be previewed together.
- Browser verification rendered five surfaces, one image card, notice, facts, ask, and a table. Search reduced rows to Targaryen/dragon; filter reduced rows to Lannister. Final reload after server restart showed five surfaces, one image, one table, table controls, and three first-page rows.
- Modal follow-up: the A2UI shape stays unchanged; the Svelte renderer now owns dialog behavior with Escape/backdrop close, focus restore, scroll lock, and Tab focus cycling.
- Modal browser check: `/chat?v=modal-js-fixed-*` opened the `ui.ask` dialog, focused the close button, locked body scroll, closed on Escape, closed on backdrop click, and closed/logged `policy.allow_once` from an action.

## Coverage Ledger

- Unit/contract: `cargo test -p capsem-ui-catalog -- --nocapture` passes.
- Integration: `cargo test -p capsem-a2ui-hyper -- --nocapture` passes.
- Regression: `cargo test -p capsem-plugin-engine -- --nocapture` passes.
- Frontend: `npm run ui:build`, `npm run typecheck`, and `npm test -- --run` pass.
- Modal follow-up frontend gate: `npm run ui:build`, `npm run typecheck`, `npm test -- --run`, and browser dialog checks pass.
- Adversarial: malformed table rows and raw renderer input rejection remain covered.
- E2E/UI: Chrome DevTools verified `/chat?v=pack01` block pack and table controls.
- Telemetry: not applicable.
- Performance: not applicable.
