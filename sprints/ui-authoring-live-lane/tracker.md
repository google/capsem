# Sprint: UI Authoring Live Lane

## Tasks

- [x] Plan sprint and proof matrix.
- [x] Add typed `ui.card` lowering.
- [x] Add typed `ui.ask` lowering.
- [x] Add Rust tests for authored card/modal programs.
- [x] Persist latest `/ui/tools/run` result in the server.
- [x] Expose latest authored result in `/ui/spec/demo`.
- [x] Render authored Agent Draft in the workbench.
- [x] Add automatic workbench refresh.
- [x] Run proof gates.
- [x] Browser verify live authoring.
- [x] Changelog.
- [x] Commit.

## Notes

- This sprint is about making Codex able to push a UI into the workbench.
  Natural-language planning still happens in the agent; the server accepts only
  structured tool calls.
- `ui.card` lowers to A2UI Basic `Card`, `Column`, `Row`, `Icon`, and `Text`
  with a `card:simple` recipe.
- `ui.ask` lowers to A2UI Basic `Modal`, `Button`, `Column`, `Row`, and `Text`
  with a `modal:basic` recipe.
- `/ui/tools/run` stores the latest result in memory; `/ui/spec/demo` exposes
  it as `authored`; the workbench renders it first as `Agent Draft`.
- Workbench polling is intentionally simple for the prototype. The production
  path should become a proper event/subscription lane.

## Coverage Ledger

- Unit/contract:
  - `cargo test -p capsem-plugin-engine -- --nocapture`
  - `cargo test -p capsem-plugin-server -- --nocapture`
- Functional:
  - `POST /ui/tools/run` with an `ui.ask` program returned `ok == true`,
    `componentCount == 12`, and `recipe.component == "modal"`.
  - `GET /ui/spec/demo` returned the latest authored draft.
  - `npm run ui:build`
- Adversarial:
  - Existing `ui_tools_reject_raw_renderer_inputs` remains passing.
- E2E/browser:
  - Chrome loaded `http://127.0.0.1:8787/?v=live-authoring`, saw
    `Agent Draft`, opened the generated modal, and found the authored prompt
    plus `Deny` and `Allow` buttons.
- Telemetry: deferred.
- Performance: deferred.
- Missing/deferred: production MCP server exposure, persistent storage, schema
  generation, and multi-draft history.
