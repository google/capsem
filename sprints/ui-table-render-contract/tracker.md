# Sprint: UI Table Render Contract

## Tasks

- [x] Write sprint plan and tracker.
- [x] Add failing renderer-drift contract for typed UI recipes.
- [x] Add typed Rust `ui.table` lowering.
- [x] Add Preline table renderer.
- [x] Add Hyper E2E contract coverage.
- [x] Render the live table in `/chat`.
- [x] Testing gate.
- [x] Changelog.
- [ ] Commit.

## Notes

- User correction: `ui.table` must not exist unless the HTML renderer knows how to render it.
- The drift gate must be general, not a one-off table assertion.
- The first focused run failed as intended: `modal` was not recipe-explicit and `table` had no renderer branch yet.
- Live `/chat` check after `POST /ui/tools/run`: browser saw one `<table>`, nine body rows, and Preline table classes.

## Coverage Ledger

- Unit/contract: `cargo test -p capsem-plugin-engine -- --nocapture` passes; covers table lowering, renderer coverage, plugin install/run, mini Capsem file UI, and catalog tests.
- Functional: `cargo test -p capsem-a2ui-hyper -- --nocapture` passes; covers Hyper table response.
- Adversarial: `ui_table_rejects_rows_with_wrong_cell_count` rejects malformed table input.
- E2E/UI: Chrome DevTools check on `http://127.0.0.1:8787/chat?v=table-contract` returned `tableCount: 1`, `rows: 9`, and `hasPrelineTableClasses: true`.
- Telemetry: missing/deferred in isolated prototype.
- Performance: missing/deferred; not relevant to this correction.
