# Sprint: TUI Control

## Active Sub-Sprint: T00

- [x] Create meta sprint and T00 plan.
- [x] Add `capsem-tui` workspace crate.
- [x] Define fixture app state and provider boundary.
- [x] Render basic standalone screen.
- [x] Add snapshot/text render proof.
- [x] Add changelog entry.
- [x] Run focused tests.
- [ ] Commit functional milestone.

## Notes

- Product correction: service/transport state is global, not per-tab.
- Per-tab indicators are lifecycle and attention state only.
- UI may only render fields exposed by typed state. If service HTTP does not
  expose a field, the UI cannot use it.
- Capsem MCP is connected but no screenshot/capture tool is exposed in the
  current tool surface.
- T00 snapshot at 100x24 confirms the basic layout and also shows light-bar
  clipping pressure for long repo/session metadata.

## Coverage Ledger

- Unit/contract: `cargo test -p capsem-tui`.
- Functional: `cargo run -p capsem-tui -- --snapshot --width 100 --height 24`.
- Adversarial: deferred until API/input parsing.
- E2E/VM: deferred until service wiring.
- Telemetry: deferred until service wiring.
- Performance: deferred until interactive frame loop work.
