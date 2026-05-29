# T00: Crate Setup And Basic Standalone Screen

Status: Done

## Scope

Create `crates/capsem-tui` as a Rust binary/library crate with:

- fixture-backed app state;
- global service/light-bar model;
- per-session tab lifecycle/attention model;
- basic Ratatui screen rendering;
- standalone terminal demo;
- non-interactive render output for tests and agent inspection.

## Non-Goals

- No real gateway/service HTTP calls yet.
- No session creation/control yet.
- No Firebase/remote transport yet.
- No real terminal attach yet.

## Design Notes

- The UI is a dumb renderer over typed state.
- Missing fields must not be inferred locally.
- The provider trait exists from day one so fixture and future HTTP providers
  use the same contract.
- Screenshot inspection through Capsem MCP is not currently available from the
  exposed MCP tools. T00 will provide buffer/text render proof first; T01 will
  decide whether to add ANSI/SVG/PNG export or a Capsem MCP capture tool.

## Done

- Workspace includes `crates/capsem-tui`.
- `cargo test -p capsem-tui` passes.
- `cargo run -p capsem-tui -- --snapshot` prints a stable standalone screen.
- `cargo run -p capsem-tui` opens an interactive fixture demo.
- The screen uses minimal chrome: no boxes, no persistent help footer, a compact
  tab strip, quiet terminal space, and one bottom status bar.

## Coverage Ledger

- Unit/contract: `cargo test -p capsem-tui`.
- Functional: `cargo run -p capsem-tui -- --snapshot --width 100 --height 24`.
- Adversarial: not applicable until input/API parsing exists.
- E2E/VM: deferred to service wiring.
- Telemetry: deferred to service wiring.
- Performance: deferred to interactive loop polish.
