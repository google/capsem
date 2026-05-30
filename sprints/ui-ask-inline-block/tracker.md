# Sprint: UI Ask Inline Block

## Tasks

- [x] Write sprint plan and tracker.
- [x] Update Rust `ui.ask` lowering and typed helper.
- [x] Add/adjust Rust and Hyper contract tests.
- [x] Add explicit Svelte/Preline ask renderer.
- [x] Browser verify `/chat`.
- [x] Testing gate.
- [x] Changelog.
- [x] Commit.

## Notes

- User correction: ask is an inline decision block in chat, not a modal.
- Compatibility: keep old `text`, `yes`, `no`, and `buttonLabel` callers working.
- `title + detail` now works; legacy `text` becomes the title when no title is
  provided, and becomes detail when title is present.
- Choices are deliberately constrained to exactly two actions.
- Browser check on `/chat?v=inline-ask-*` showed `Policy decision`, optional
  detail text, two inline buttons, no dialog, and `policy-choice.yes` logged on
  click.

## Coverage Ledger

- Unit/contract: `cargo test -p capsem-ui-catalog -- --nocapture` passes.
- Functional: `cargo test -p capsem-a2ui-hyper -- --nocapture` passes.
- Frontend: `npm run ui:build`, `npm run typecheck`, and `npm test -- --run` pass.
- Adversarial: missing ask prompt and three-choice ask are rejected.
- E2E/UI: Chrome DevTools verified inline `/chat` rendering and action logging.
- Telemetry: not applicable.
- Performance: not applicable.
- Missing/deferred: none.
