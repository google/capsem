# TUI Empty State

## Goal

Make first launch understandable when the gateway has no VMs: the TUI should
show a branded empty-state panel with the CAPSEM ASCII/wordmark treatment, the
existing create-session form, and the core shortcuts instead of immediately
opening the create modal.

## Decisions

- Keep the existing create-session draft/profile selection logic in the empty
  panel; plain Enter creates the selected profile from the empty screen.
- Do not invent profiles or create defaults; the create modal remains blocked
  if `/profiles` is unavailable.
- Keep service-offline startup distinct: unavailable service still prompts to
  start Capsem before offering session creation.
- Replace unclear test-only action-error wording with a user-comprehensible
  service error.

## Files

- `crates/capsem-tui/src/app.rs`
- `crates/capsem-tui/src/ui.rs`
- `crates/capsem-tui/src/tests.rs`
- `CHANGELOG.md`

## Done

- Empty online session state renders the branded panel and shortcut list.
- Enter creates the selected profile directly from the empty state.
- Existing profile-unavailable guard still blocks create confirmation.
- TUI tests and check pass.
