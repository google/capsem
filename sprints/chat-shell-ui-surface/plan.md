# Chat Shell UI Surface Sprint

## Goal

Show the authored A2UI component inside a chat application shell. The user
should see a normal chat thread with an assistant message that contains the
rendered component, proving the same UI tool output can land in a user-facing
conversation surface.

## Product Slice

The workbench already receives a live `Agent Draft` from `/ui/tools/run`.
This sprint changes the primary view from a standalone preview panel to a chat
shell:

```text
user request
  -> assistant response
  -> rendered A2UI component inside assistant message
  -> inspector remains available on the right
```

## Decisions

- Keep the right inspector. We still need A2UI/tool/template visibility while
  the UI system is forming.
- Render the selected workbench item inside the chat message, with Agent Draft
  selected first when present.
- Use Preline tokens and component recipes only. No raw colors or component
  library drift.
- This sprint does not add a real prompt parser or chat backend. It is a shell
  proving placement and rendering.

## Files

- `ui-preview/src/App.svelte`
- `CHANGELOG.md`
- `sprints/chat-shell-ui-surface/tracker.md`

## Done Means

- The browser shows a chat shell as the primary surface.
- The authored Agent Draft component appears inside an assistant message.
- The inspector still shows A2UI/tools/template evidence.
- The app remains responsive on desktop-width browser.

## Proof Matrix

- Unit/contract: existing Rust and frontend tests remain green.
- Functional: post a card draft through `/ui/tools/run`; workbench renders it
  inside the chat shell.
- E2E/browser: Chrome confirms `Capsem Chat`, `Agent Draft`, and the authored
  component text are visible.
- Telemetry/performance: deferred.
