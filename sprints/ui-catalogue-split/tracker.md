# Sprint: UI Catalogue Split

## Tasks

- [x] Write sprint plan and tracker.
- [x] Create standalone `capsem-ui-catalog` crate.
- [x] Move UI catalogue/tests out of plugin engine.
- [x] Rewire Hyper/server to depend on catalogue directly.
- [x] Keep compatibility re-exports from plugin engine.
- [x] Add mainline porting notes.
- [x] Verify `/chat` still renders authored UI.
- [x] Testing gate.
- [x] Changelog.
- [x] Commit.

## Notes

- This is intentionally not a WASM/plugin contract sprint. It extracts the UI contract so it can land first.
- Porting notes live in `sprints/ui-catalogue-split/porting.md`; crate-level notes live in `crates/capsem-ui-catalog/README.md`.
- Browser verification after split: `/chat?v=catalogue-split` rendered one table, nine body rows, theme select, and dark switch.

## Coverage Ledger

- Unit/contract: `cargo test -p capsem-ui-catalog -- --nocapture` passes.
- Integration: `cargo test -p capsem-a2ui-hyper -- --nocapture` passes.
- Regression: `cargo test -p capsem-plugin-engine -- --nocapture` passes.
- E2E/UI: Chrome DevTools verified `/chat` still renders authored table after the crate split.
- Frontend: `npm run ui:build`, `npm run typecheck`, and `npm test -- --run` pass.
- Telemetry: not applicable.
- Performance: not applicable.
