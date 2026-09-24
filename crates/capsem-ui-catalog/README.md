# capsem-ui-catalog

Standalone Rust ownership for Capsem's A2UI-based UI catalogue.

This crate is intentionally independent of the WASM/plugin runtime. It can land
ahead of the full plugin contract work and be reused by:

- chat/model-generated UI;
- MCP or gateway UI tools;
- plugin callbacks that emit structured UI;
- future Capsem app surfaces.

## Modules

- `ui`: A2UI v0.9 Basic message model, typed helper builders, and message
  validation against `a2ui-types`.
- `ui_tools`: Capsem-authored tool program API (`ui.alert`, `ui.notice`,
  `ui.card`, `ui.facts`, `ui.table`, `ui.ask`), recipe metadata,
  renderer-drift checks, and Preline template binding checks.
- `contract_matrix`: machine-readable A2UI, Capsem block, rich artifact, and
  local-tool coverage tables used to freeze pack boundaries.
- `editable_state`: Loro-backed editable UI/artifact audit fixture for cards,
  slide decks, sheets, and website forms.

## Pack 01 Blocks

- `ui.alert`: message, tone, variant.
- `ui.notice`: title, message, tone, optional actions.
- `ui.card`: title, description, optional image, optional link, optional
  actions.
- `ui.facts`: titled key/value facts panel.
- `ui.table`: columns, rows, search, first-column filtering, and pagination
  metadata.
- `ui.ask`: yes/no defaults or typed choice actions.

## Boundary

The wire format stays A2UI Basic. Capsem recipe metadata selects the renderer
adapter; it must not contain raw HTML, CSS classes, or executable renderer code.

`capsem-plugin-engine` may re-export this crate for compatibility, but new code
should import `capsem_ui_catalog` directly.

## Port Checklist

1. Add `crates/capsem-ui-catalog` to the main workspace.
2. Add workspace dependency `capsem-ui-catalog = { path = "crates/capsem-ui-catalog" }`.
3. Move `templates/capsem-ui/**` with the crate or keep the same repo-relative
   path used by `check_template_dir` tests.
4. Point gateway/server/chat UI endpoints at `capsem_ui_catalog::ui_tools`.
5. Keep `capsem-plugin-engine` compatibility re-exports only as a transition
   shim.
6. Port the Svelte renderer and keep `typed_ui_recipe_components_have_explicit_svelte_renderers`
   passing so Rust tools cannot drift away from HTML rendering.
