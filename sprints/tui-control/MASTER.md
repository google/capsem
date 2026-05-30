# Capsem TUI Control Meta Sprint

Status: In Progress

## Goal

Build a Rust-native terminal control plane for Capsem that feels like switching
between lightweight VM/agent desktops, not operating a dashboard. The TUI is a
thin client over typed state and actions exposed by Capsem service/gateway APIs.

## Product Contract

- Global service state belongs in the light/status bar.
- Per-session/tab state shows lifecycle and attention only: idle, suspended,
  working, waiting for input, approval required, failed, bell, stale.
- The TUI must not infer unavailable state. If a field is missing from the
  service HTTP model, it becomes a service/API requirement.
- Basic UI can run standalone with fixture state before real gateway wiring.
- Full stats, session picker, help, and new-session flows are overlays/screens.

## Sub-Sprints

| ID | Status | Scope | Proof |
| --- | --- | --- | --- |
| T00 | Done | Crate setup and standalone fixture screen | `cargo test -p capsem-tui`; snapshot command |
| T01 | Done | Terminal screenshot/snapshot proof path | `--snapshot-svg`; rendered PNG inspection |
| T02 | Done | Multiple desktop tabs and per-session indicators | render tests for active/attention states |
| T03 | Done | Keyboard controls and focus/modal ownership | key-sequence tests |
| T04 | Done | Help, full statistics, and new-session screens | overlay render tests |
| T05 | Done | Home/resume screen with profile/session list | fixture render tests |
| T06 | Done | Typed HTTP gateway model inventory and API gaps | `/status` schema mapped into TUI model |
| T07 | Done | Wire installed gateway read-only state | HTTP provider test + live snapshot |
| T08 | Done | Safe service control actions | confirmation/action tests |
| T09 | Not Started | Remote transport readiness | reconnect/event cursor tests |
| T10 | Done | Active terminal WebSocket surface | terminal buffer/input tests + live two-session gateway proof |

## Current Decision

The standalone shell is now wired to the installed Capsem HTTP
gateway. Default mode discovers the installed gateway port from runtime files,
falls back to `http://127.0.0.1:19222`, fetches `/token`, and then polls
authenticated `GET /status`. Safe mutating actions go through the same gateway
with a confirmation overlay and a background worker. `--fixture` keeps the
two-session demo path for visual iteration; `--gateway-url` turns connection
failures into explicit errors for focused gateway testing. The active terminal
WebSocket path is live-proven against MCP-created `tui-proof-a` and
`tui-proof-b`; healthy `profile_status=current` sessions no longer render stale
attention markers.

## T00 Closeout

- Added `crates/capsem-tui`.
- Added fixture state with global service health and per-session indicators.
- Added a basic Ratatui screen and deterministic `--snapshot` output.
- Deferred real screenshot export/CAPSEM MCP capture to T01 because the current
  exposed Capsem MCP tool surface does not include terminal screenshot capture.

## T01-T03 Closeout

- Added `--snapshot-svg` style-preserving export for visual proof.
- Reworked the standalone shell into a tmux-like single status bar with global
  service state on the left, numbered session tabs in the center, and active
  session stats on the right.
- Added a typed app controller for session switching.
- Kept plain `q` and Ctrl-C available for the agent/terminal stream. The TUI
  shell owns Alt chords: `Alt+Left/Right`, `Alt+1..9`,
  `Alt+n/f/r/s/c/t/d`, `Alt+q`, `Alt+?`, `Alt+i`, and `Alt+l`.
- Added `just dev-tui` for direct local playback.

## T04-T05 Closeout

- Added hidden overlays for help, active-session statistics, and session/home
  list.
- Kept the normal terminal surface clean; overlays only appear through function
  keys and toggle back off with the same key.
- Scoped the home screen to existing sessions for this slice. New-session
  creation remains part of the later safe-action sprint because it mutates
  service state.

## T06-T07 Closeout

- Inventoried the existing gateway status model instead of adding a parallel
  API: `StatusResponse { vms }` already carries ID, name, status, profile,
  uptime, token/cost counters, policy-deny counters, and file event/request
  counters.
- Added a typed `GatewayProvider` that reads the installed HTTP gateway.
- Mapped service status into TUI lifecycle state: running, suspended, stopped,
  failed/defunct.
- Mapped existing gateway-exposed deny and stale profile status into attention
  markers.
- Added periodic interactive refresh while preserving the selected tab when it
  still exists after reload.
- Added active-session terminal WebSocket wiring through the gateway:
  `/token`, `/terminal/{id}?token=...`, resize messages, terminal input
  forwarding, and output buffering for the Ratatui surface.
- Added `capsem_terminal_snapshot` to the host MCP server so agents can inspect
  session terminal/log state without needing an image-capable screenshot tool.
- Added confirmed create/resume/suspend/stop/delete actions through the
  installed gateway, with background execution so long service operations do
  not block terminal rendering.
- Proved the installed gateway path with two live persistent sessions created
  through Capsem MCP. `capsem-tui --snapshot` renders both sessions and a direct
  gateway WebSocket command returned `TUI_WS_PROOF_A` from `tui-proof-a`.
- Replaced the temporary terminal text parser with `vt100`, preserving xterm
  screen state, SGR colors, and text attributes. Client-side adjacent output
  coalescing and dirty-frame redraws now mirror the existing `capsem shell`
  speed contract instead of repainting on every loop.
- Tightened interactive control polish: help opens on `Alt+?`, overlays render
  as Ratatui modal blocks, service latency renders as a glued `####ms●`
  segment, and active terminal geometry is resent whenever the real terminal
  size changes.
- Simplified human-facing tab colors: selected VM is yellow, every other VM tab
  is blue. Modal overlays now close with `Esc`, own normal keys while visible,
  and release VM input forwarding immediately after close.
- Kept richer missing state explicit for future API work: waiting-for-input,
  terminal bell, per-session repo/path metadata, security/enforcement/detection
  totals, and event cursor semantics are not invented by the TUI.
- Reproduced and fixed the local latency stack under an 8-live-VM endpoint
  gate: `/list` stays in-memory, process metrics snapshots no longer scan
  session directories, raw session DB queries no longer pay a 100ms watchdog
  floor, `/stats` has an empty/read-only fast path, and policy-context exports
  dedupe by security event before fixture projection.
- Fixed inactive-session handling after the proof VMs were stopped: stopped,
  suspended, and failed sessions now render a greyed tab plus a centered
  `Press Enter to resume` prompt, Enter invokes resume for the active inactive
  session, and the terminal WebSocket bridge disconnects instead of reconnecting
  to stopped VM sockets.
- Added a far-right `help: alt+?` hint after active-session statistics so help
  is discoverable without competing with service health or `Alt+s` suspend.
- Corrected `Alt+n` from one-key ephemeral provisioning into a profile-aware
  new-session modal. The flow pre-fills the next unused `tmp-*` name, supports
  typing/backspace, lets Up/Down choose a live `/profiles` entry, and sends a
  named persistent `/provision` request with the selected `profile_id`.
- Added `Alt+f` as a fork modal. The flow pre-fills the next unused
  `<source>-fork` name, supports typing/backspace, and sends authenticated
  `POST /fork/{id}` with the chosen `name`.
- Split `Alt+s` and `Alt+c`: `Alt+s` is suspend, while `Alt+c` is the
  checkpoint/save affordance routed through the current suspend endpoint until
  the service exposes a separate checkpoint-only API.
- Reworked help, session list, and session info as structured tables. `Alt+l`
  is the primary session-list chord, `Alt+i` opens active-session info, and
  create/fork modals now visibly highlight the active input and selected
  profile row.

## Testing Gate

- Unit/contract: required for state, render, confirmation, and action wiring.
- Functional: standalone demo, text snapshot, and SVG render output.
- Adversarial: malformed gateway status, authenticated provider parsing, and
  action error propagation.
- E2E/VM: live empty-service snapshot covered; live multi-VM terminal session
  proof covered with MCP-created `tui-proof-a` and `tui-proof-b`, plus installed
  gateway terminal WebSocket shell output from `tui-proof-a`.
- Telemetry: mapped from current counters; event stream/cursor semantics remain
  open.
- Performance: 8-live-VM endpoint benchmark is active in the serial benchmark
  gate. Latest release-binary arm64 proof has `/list` p95 0.335ms, `/stats`
  p95 0.798ms, slowest per-VM read `/files` p95 2.491ms, and gateway
  `/status` p95 0.223ms. Concurrent boot pressure remains a separate follow-up
  because endpoint speed should not depend on parallel provisioning setup.
- Regression: `cargo test -p capsem-tui` covers stopped-session prompt,
  greyed inactive tab tone, Enter-to-resume behavior, and the named fork
  modal/action path, plus `Alt+l` sessions, `Alt+i` session info, and `Alt+c`
  checkpoint. Live snapshot against the installed stopped `tui-proof-*`
  sessions shows the prompt instead of a blank pane.
- UI polish: `cargo test -p capsem-tui` and snapshot output cover the
  right-side `help: alt+?` status-bar hint after session stats and
  focused-field highlighting, including the no-active-session status-bar path.
- New-session regression: `cargo test -p capsem-tui` covers create-modal
  rendering, profile selection, name editing, and authenticated named-profile
  provision request payloads.
- Fork regression: `cargo test -p capsem-tui` covers fork-modal rendering,
  name editing, help discoverability, and authenticated gateway fork payloads.
- Checkpoint regression: `cargo test -p capsem-tui` covers `Alt+c` confirmation
  and the authenticated checkpoint request over the current suspend endpoint.
