# Sprint: TUI Control

## Active Sub-Sprint: T10

- [x] Create meta sprint and T00 plan.
- [x] Add `capsem-tui` workspace crate.
- [x] Define fixture app state and provider boundary.
- [x] Render basic standalone screen.
- [x] Add snapshot/text render proof.
- [x] Add changelog entry.
- [x] Run focused tests.
- [x] Add SVG snapshot proof path.
- [x] Rework status bar to tmux-style left/center/right layout.
- [x] Add two-session standalone fixture for local playback.
- [x] Add keyboard session switching without capturing plain `q`.
- [x] Add `just dev-tui` recipe.
- [x] Add hidden help, stats, and sessions overlays.
- [x] Inventory existing gateway `/status` model for TUI state.
- [x] Add typed gateway provider over installed HTTP gateway.
- [x] Map lifecycle, attention, uptime, token, cost, job, and event counters.
- [x] Add malformed status and authenticated HTTP provider tests.
- [x] Refresh live state periodically in interactive mode.
- [x] Add active-session terminal WebSocket client through the gateway.
- [x] Forward terminal input keys while keeping app navigation shortcuts owned
      by the shell.
- [x] Preserve plain `q` and Ctrl-C for the agent/terminal stream.
- [x] Render active terminal output in the main Ratatui surface.
- [x] Add terminal buffer, ANSI cleanup, and key encoding tests.
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
- Product correction after visual review: removed boxes and persistent help,
  moved global service latency plus cumulative session status into the single
  bottom bar, and kept tabs as a compact sliding strip.
- Product correction after tmux reference review: removed aggregate VM status
  counts from the left, kept only service health/latency, colored only the
  active tab and attention tabs, and tied tab label color to the number tone.
- Keyboard policy: plain `q` and Ctrl-C belong to the agent/terminal stream, so
  the standalone shell exits via F10, Ctrl-Esc, or Cmd-Q if the terminal emits
  it.
- Default `dev-tui` reads the installed HTTP gateway when available. It uses
  `CAPSEM_GATEWAY_URL` when set, otherwise the installed runtime
  `gateway.port`, otherwise `http://127.0.0.1:19222`.
- `--fixture` forces the two-session visual demo, and `--gateway-url <url>` is
  strict for gateway debugging.
- Current service state on this machine responds but has no live sessions, so
  the live snapshot correctly renders `no session`.
- API gaps still open for later sub-sprints: waiting-for-input status, terminal
  bell, per-session repo/path metadata, security/enforcement/detection totals
  on gateway `/status`, event cursoring, and remote transport latency/error
  details.
- Terminal WebSocket slice is intentionally active-session only for now. It
  connects the selected tab and reconnects when the selected tab changes; idle
  background session multiplexing belongs in the later virtual-desktop sprint.
- New-session creation is deliberately not in the hidden sessions overlay yet
  because it mutates service state and belongs with safe action confirmation.

## Coverage Ledger

- Unit/contract: `cargo test -p capsem-tui`.
- Functional: `cargo run -p capsem-tui -- --snapshot --width 100 --height 24`;
  `cargo run -p capsem-tui -- --fixture --snapshot --width 120 --height 30`;
  `cargo run -p capsem-tui -- --fixture --snapshot-svg --width 120 --height 30`;
  `just dev-tui`.
- Gateway wiring: `GatewayProvider::load_async` authenticated HTTP mock test
  plus live local snapshot through the installed gateway.
- Terminal wiring: `TerminalSurface` output/ANSI tests and key-encoding tests.
- Overlay wiring: function-key state tests and stats overlay render test.
- Adversarial: malformed gateway status mapping.
- E2E/VM: live multi-session terminal proof still open; current installed
  gateway has no live sessions.
- Telemetry: current gateway `/status` counters mapped; event-stream semantics
  still open.
- Performance: not measured yet.
