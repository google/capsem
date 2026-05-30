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
- [x] Add hidden help, session-info, and sessions overlays.
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
- [x] Replace the hand-rolled ANSI text flattener with a VT/xterm parser that
      preserves terminal colors and text attributes.
- [x] Add client-side terminal output coalescing and dirty-frame redraws.
- [x] Stabilize service latency width in the bottom bar.
- [x] Replace terminal-dependent Cmd/Ctrl navigation guesses with an app-owned
      Alt namespace for shell controls.
- [x] Add `capsem_terminal_snapshot` MCP tool for session terminal inspection.
- [x] Add confirmed create/resume/suspend/stop/delete actions through the
      installed HTTP gateway.
- [x] Add named fork action through the installed HTTP gateway.
- [x] Split `Alt+s` suspend from `Alt+c` checkpoint/save.
- [x] Rework help, session list, and session info into structured readable
      tables.
- [x] Highlight active modal fields and selected profile rows.
- [x] Run live installed-gateway empty-service snapshot.
- [x] Run live two-session terminal proof.
- [x] Commit functional milestone.

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
- Keyboard policy: plain `q` and Ctrl-C belong to the agent/terminal stream.
  The TUI shell exits with `Alt+q` and keeps app-owned controls under Alt.
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
- MCP terminal inspection is now a text snapshot from service logs, not a
  bitmap screenshot. It is enough for agent debugging and works through the
  existing service log contract.
- Safe service actions are now active behind a confirmation overlay. `Alt+n`
  opens the new-session dialog, `Alt+f` opens the fork dialog, `Alt+r` resumes
  stopped/suspended sessions, `Alt+s` suspends the active session, `Alt+c`
  checkpoints/saves it through the current suspend endpoint, `Alt+t` stops it,
  and `Alt+d` deletes it.
  Action calls run on a
  background worker so long suspend/stop/provision paths do not freeze terminal
  rendering.
- Live VM proof is unblocked. MCP `capsem_list` reports asset health ready and
  two running persistent proof sessions, `tui-proof-a` and `tui-proof-b`, on
  `everyday-work@2026.0529.5`.
- Live snapshot now renders both proof sessions without false attention markers:
  `cargo run -p capsem-tui -- --snapshot --width 120 --height 30`.
- Live terminal WebSocket proof through the installed HTTP gateway succeeded
  against `tui-proof-a` and returned `TUI_WS_PROOF_A` from the VM shell.
- Fixed `profile_status=current` handling so healthy profile pins do not render
  stale/attention markers.
- Terminal rendering now uses `vt100` for screen state and SGR styles. The TUI
  no longer keeps a parallel ANSI parser, coalesces adjacent terminal output
  events before parsing, and draws only when state/input/output marks the frame
  dirty.
- Keyboard input is read by a blocking input reader thread instead of
  `crossterm::event::poll`; the WebSocket path remains async and event-driven.
- Service latency reserves four digits before `ms`, preventing the center tab
  strip from shifting when latency changes between one and four digits.
- Service latency now renders as `####ms●`, with the status dot glued to the
  reserved latency field so it reads as one service-health segment.
- Navigation is now app-owned: `Alt+Left/Right` switches sessions and
  `Alt+1..9` jumps by tab number. `Alt+n/f/r/s/c/t/d`, `Alt+q`,
  `Alt+?`, `Alt+i`, and `Alt+l` cover shell actions, exit, help, session
  info, and session list.
- Tab colors now use one semantic: selected is yellow, every other VM tab is
  blue. Bell/attention state keeps its text marker but no longer changes the
  tab color.
- Interactive terminal resize now tracks the active session and geometry
  together, so a pure terminal resize resends the guest PTY size even when the
  selected VM did not change.
- Help, session info, sessions, and confirmation overlays now use Ratatui
  `Clear` and bordered modal blocks instead of drawing loose text over
  terminal output.
- Help, session info, and sessions are real modals: `Esc` closes them, visible
  modals consume normal keys, and plain VM input forwards again immediately
  after close. Key-release events are ignored in the interactive loop.
- `just dev-tui` documents the same Alt-only shell contract inline so local
  playback and installed usage do not drift.
- MCP triage for `tui-proof-a` found no session-level failures. Host triage
  still shows stale gateway terminal reconnect errors for the removed
  `crafty-panda` socket, which are unrelated to the proof sessions.
- Bug fix: status/metrics IPC polling was leaking per-connection writer and
  lifecycle-forwarder tasks in `capsem-process`, eventually exhausting file
  descriptors under the two-VM TUI proof. Teardown now aborts every
  per-connection helper.
- Live fd stress after install: 150 service `/list` refreshes across
  `tui-proof-a` and `tui-proof-b` kept process fd counts flat at 39 and 40.
- Local latency diagnosis: the original two-VM 4-8ms reading was real service
  work, not a UI display problem. `/list` was still calling per-VM live metrics,
  `capsem-process` metrics snapshots recursively scanned session directories,
  and raw session DB queries paid a fixed 100ms watchdog-thread floor.
- TUI latency fix: gateway refreshes now reuse the HTTP client and cached
  gateway token while preserving the freshly measured latency value. The
  service hot paths now keep `/list` in-memory, keep metrics snapshots
  process-owned, use SQLite progress handlers for raw query timeouts, skip
  `/stats` schema creation on read, and dedupe policy-context exports by
  security event.
- Endpoint latency gate: 8 live temporary VMs now cover global service reads,
  per-VM info/logs/history/files/policy-context reads, and gateway
  health/token/status reads. Latest release-binary arm64 run: `/list` p95
  0.335ms, `/stats` p95 0.798ms, slowest per-VM endpoint `/files` p95
  2.491ms, gateway `/status` p95 0.223ms.
- Boot pressure follow-up: an early 4-way parallel benchmark setup run hit one
  `wait_exec_ready` miss before latency measurement. Sequentially provisioning
  8 live VMs is stable, so endpoint latency remains gated separately from a
  future concurrent-boot pressure test.
- Stopped-session bug: stopped sessions were still selectable tabs, but the
  main pane only rendered live terminal buffers and the WebSocket manager still
  tried to connect. Stopped/suspended/failed tabs now grey out, the pane shows
  `Press Enter to resume`, Enter invokes resume for the active inactive
  session, and the terminal bridge disconnects from inactive tabs.
- Discoverability polish: the persistent status segment now ends with
  `help: alt+?` after active-session statistics. The full command list remains
  in the help overlay.
- New-session flow correction: `Alt+n` now opens a `new session` modal instead
  of immediately provisioning an ephemeral VM. The modal pre-fills the name as
  the next unused `tmp-*`, lets the user type a name, lets Up/Down choose from
  the live `/profiles` list, and provisions a named persistent session with
  the selected `profile_id`.
- Fork flow correction: `Alt+f` now opens a `fork session` modal, pre-fills
  the next unused `<source>-fork` name, lets the user type/backspace, and sends
  `POST /fork/{id}` with the chosen `name`.
- Usability correction: full help is now a structured table, `Alt+l` opens the
  session list table, `Alt+i` opens session info, and create/fork modals
  highlight the active input plus the selected profile row.

## Coverage Ledger

- Unit/contract: `cargo test -p capsem-tui` (36 tests), including
  stopped-session resume prompt, grey tab, Enter-to-resume coverage, and the
  right-side `help: alt+?` status-bar hint after session stats, plus the create
  modal profile/name flow, selected-field highlighting, named fork modal/action
  coverage, `Alt+l` sessions table, `Alt+i` session info, and `Alt+c`
  checkpoint.
- TUI latency/provider: `cargo test -p capsem-tui` (36 tests), including
  token reuse, live profile-list refresh, named fork request payloads,
  checkpoint-over-suspend payloads, and raw local latency preservation
  coverage.
- Process IPC: `cargo test -p capsem-process` (120 tests), including
  `connection_teardown_aborts_writer_and_lifecycle_tasks`.
- Service/core/logger hot paths: `cargo test -p capsem-service`,
  `cargo test -p capsem-core session`, and `cargo test -p capsem-logger`.
- Endpoint benchmark: `CAPSEM_ASSETS_DIR="$HOME/.capsem/assets" uv run python
  -m pytest tests/capsem-serial/test_endpoint_latency_benchmark.py -xvs
  --tb=short`.
- Formatting: `cargo fmt -p capsem-tui -- --check`.
- Process formatting: `cargo fmt -p capsem-process -- --check`.
- Functional: `cargo run -p capsem-tui -- --snapshot --width 100 --height 24`;
  `cargo run -p capsem-tui -- --fixture --snapshot --width 120 --height 30`;
  `cargo run -p capsem-tui -- --fixture --snapshot-svg --width 120 --height 30`;
  `cargo run -p capsem-tui -- --snapshot --width 120 --height 30` against
  the installed stopped proof sessions; `just dev-tui`.
- Gateway wiring: `GatewayProvider::load_async` authenticated HTTP mock test
  plus live local snapshot through the installed gateway.
- Service actions: confirmed action key tests plus authenticated mock gateway
  tests for successful stop, named profile create, named fork, checkpoint, and
  surfaced service error bodies.
- Terminal wiring: `TerminalSurface` output, xterm color/style preservation,
  adjacent output coalescing, and key-encoding tests.
- MCP wiring: `capsem_terminal_snapshot` router registration and rendering
  tests.
- Overlay wiring: function-key state tests and session-info overlay render
  test.
- Adversarial: malformed gateway status mapping; action error response body
  surfaced to the status bar instead of being swallowed.
- E2E/VM: live installed-gateway empty-service snapshot works; live
  multi-session terminal proof works with MCP-created `tui-proof-a` and
  `tui-proof-b`; installed gateway terminal WebSocket returned VM shell output
  from `tui-proof-a`; post-fix installed fd stress held both VM process fd
  counts flat through repeated service metrics polling.
- Telemetry: current gateway `/status` counters mapped; event-stream semantics
  still open.
- Performance: 8-live-VM endpoint gate passes with `/list` sub-ms, `/stats`
  under 1ms p95 on release binaries, all per-VM reads under the 12ms p95 gate,
  and gateway `/status` around 0.22ms p95.
