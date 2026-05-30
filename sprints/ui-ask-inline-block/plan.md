# UI Ask Inline Block Sprint

## Goal

Make `ui.ask` a chat-native inline decision block instead of a modal launcher.
The user should see the title, optional detailed text, and action buttons
directly in the generated UI.

## Decisions

- Keep the wire format A2UI Basic.
- Lower `ui.ask` to a `Card` containing `Text` and `Button` components.
- Use `recipe.component = "ask"` and `variant = "inline"` so the Svelte
  renderer has an explicit adapter.
- Treat `title` as the primary prompt. If only legacy `text` is provided,
  promote it to the title for compatibility.
- Treat `detail` as the preferred optional body text, with legacy `text`
  becoming detail when a title is also provided.
- Keep `buttonLabel` accepted but ignored; opening a modal is no longer the
  contract.

## Files

- `crates/capsem-ui-catalog/src/ui_tools.rs`
- `crates/capsem-ui-catalog/src/ui.rs`
- `crates/capsem-ui-catalog/tests/ui_tools.rs`
- `crates/capsem-a2ui-hyper/tests/a2ui_http.rs`
- `crates/capsem-plugin-server/src/ui_preview.rs`
- `ui-preview/src/A2Node.svelte`
- `CHANGELOG.md`

## Done

- `ui.ask` no longer emits A2UI `Modal` or an open button.
- Inline ask supports title, optional detail text, and actions.
- Rust/Hyper/frontend tests pass.
- Browser verifies `/chat` shows the inline decision block and buttons log
  actions without opening a modal.

## Proof Matrix

- Unit/contract: `cargo test -p capsem-ui-catalog -- --nocapture`.
- Integration: `cargo test -p capsem-a2ui-hyper -- --nocapture`.
- Frontend: `npm run ui:build`, `npm run typecheck`, `npm test -- --run`.
- E2E/UI: browser verifies `/chat` inline ask rendering and action click.
- Adversarial: ask with neither `title` nor `text` rejects.
- Telemetry: not applicable.
- Performance: not applicable.
