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
| T01 | Not Started | Terminal screenshot/snapshot proof path | buffer snapshots; screenshot/export strategy |
| T02 | Not Started | Multiple desktop tabs and per-session indicators | render tests for active/attention states |
| T03 | Not Started | Keyboard controls and focus/modal ownership | key-sequence tests |
| T04 | Not Started | Help, full statistics, and new-session screens | screen render tests |
| T05 | Not Started | Home/resume screen with profile/session list | fixture render tests |
| T06 | Not Started | Typed HTTP/service model inventory and API gaps | service schema gap doc |
| T07 | Not Started | Wire local gateway/service read-only state | fake + live gateway tests |
| T08 | Not Started | Safe service control actions | confirmation/action tests |
| T09 | Not Started | Remote transport readiness | reconnect/event cursor tests |

## Current Decision

Wire a provider boundary early, but do not wire real Capsem HTTP behavior until
the standalone shell, tab model, keyboard model, and overlays are testable. The
first crate uses fixture state through the same interface later used by HTTP.

## T00 Closeout

- Added `crates/capsem-tui`.
- Added fixture state with global service health and per-session indicators.
- Added a basic Ratatui screen and deterministic `--snapshot` output.
- Deferred real screenshot export/CAPSEM MCP capture to T01 because the current
  exposed Capsem MCP tool surface does not include terminal screenshot capture.

## Testing Gate

- Unit/contract: required for state and render logic.
- Functional: standalone demo and text/snapshot render output.
- Adversarial: malformed/missing fields once the HTTP model exists.
- E2E/VM: deferred until service wiring begins.
- Telemetry: deferred until service wiring begins.
- Performance: frame/render timing deferred until interactive loop exists.
