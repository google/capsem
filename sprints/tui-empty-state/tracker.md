# Sprint: TUI Empty State

## Tasks

- [x] Replace automatic create modal on empty online state
- [x] Render branded empty-state panel with create action and shortcuts
- [x] Remove unclear test/user-facing error wording
- [x] Update changelog
- [x] Run TUI tests and check
- [x] Commit

## Notes

- Discovery: empty online state is currently forced through
  `sync_empty_state_prompt -> open_create`, so first launch starts inside a
  modal.
- Changed approach: empty online state now remains overlay-free and uses the
  main terminal area for the first-launch panel; Enter/Alt+n opens the modal.
- Correction: CAPSEM ASCII art is explicitly kept out of the create modal; it
  belongs only to the empty screen.
- Correction: the empty screen now reuses the create-session form/draft logic
  inline; Enter creates the selected profile without opening a modal.

## Coverage Ledger

- Unit/contract: added `capsem-tui` app assertions for empty-state overlay,
  inline profile selection, and Enter-to-create behavior.
- Functional: added rendered snapshot assertions for CAPSEM branding, create
  form/profile rows, and first-launch shortcuts.
- Functional: added a modal snapshot assertion that the create dialog does not
  render the CAPSEM ASCII art.
- Adversarial: profile-unavailable create guard remains covered through the
  modal after Enter opens it.
- E2E/VM: not required; TUI state/rendering only.
- Telemetry: not touched.
- Performance: not touched.
- Missing/deferred: none planned.

## Verification

- `cargo test -p capsem-tui`
- `cargo check -p capsem-tui`
