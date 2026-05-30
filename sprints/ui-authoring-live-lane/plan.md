# UI Authoring Live Lane Sprint

## Goal

Make the isolated UI workbench usable as an agent target. After this sprint,
Codex can receive a UI request in chat, translate it into structured Rust UI
tool calls, POST that program to the server, and the open workbench will show
the authored surface without hand-editing JSON or restarting the server.

## Product Slice

The current workbench proves the self-use path with a fixed acceptance program.
That is not enough. The next slice adds a live "Agent Draft" lane:

```text
Codex chat request
  -> structured UI tool program
  -> POST /ui/tools/run
  -> Rust validates and stores latest result in memory
  -> GET /ui/spec/demo includes latest authored result
  -> Svelte workbench polls and renders Agent Draft first
```

## Decisions

- Keep this isolated in the prototype crate. No Capsem production wiring.
- Store only the latest authored result in memory. Persistence belongs to the
  later plugin/session telemetry design.
- Continue rejecting HTML, CSS class strings, scripts, and renderer code.
- Add typed convenience tools only when they lower to A2UI Basic messages:
  `ui.alert`, `ui.card`, and `ui.ask`.
- Use the same Svelte renderer and inspector for authored results and examples.

## Files

- `crates/capsem-plugin-engine/src/ui_tools.rs`
- `crates/capsem-plugin-engine/tests/ui_tools.rs`
- `crates/capsem-plugin-server/src/main.rs`
- `crates/capsem-plugin-server/src/ui_tools.rs`
- `crates/capsem-plugin-server/src/ui_preview.rs`
- `ui-preview/src/App.svelte`
- `ui-preview/src/a2ui.ts`
- `CHANGELOG.md`
- `sprints/ui-authoring-live-lane/tracker.md`

## Done Means

- `POST /ui/tools/run` validates and stores the latest authored UI result.
- `GET /ui/tools/latest` returns the latest authored result.
- `GET /ui/spec/demo` includes the latest authored result for the workbench.
- The workbench renders "Agent Draft" first when a result exists.
- The workbench refreshes automatically while open.
- `ui.card` and `ui.ask` are available as typed convenience tools.
- Tests prove live authoring can build a modal/card surface and reject raw
  renderer input.

## Proof Matrix

- Unit/contract: Rust tests for `ui.card`, `ui.ask`, validation, and rejection.
- Functional: POST a sample program, fetch `/ui/spec/demo`, confirm the draft
  surface is present and renderable.
- Adversarial: raw HTML/class input remains rejected.
- E2E: browser shows the authored draft after POST.
- Telemetry: deferred.
- Performance: deferred; this is a correctness sprint.
