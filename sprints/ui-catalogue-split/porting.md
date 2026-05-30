# Mainline Porting Notes

This sprint separates the UI catalogue from the plugin/WASM runtime so another
agent can port it into main Capsem without taking the full plugin stack.

## What To Port

- `crates/capsem-ui-catalog/**`
- workspace member/dependency changes in `Cargo.toml`
- direct imports in the UI-serving code:
  - `capsem_ui_catalog::ui`
  - `capsem_ui_catalog::ui_tools`
- frontend renderer pieces that consume the catalogue output:
  - `ui-preview/src/a2ui.ts`
  - `ui-preview/src/A2Node.svelte`
  - `ui-preview/src/ChatPage.svelte`
  - `ui-preview/src/style.css`
- `templates/capsem-ui/**`
- semantic-token test: `test/ui-semantic-tokens.test.ts`

## What Not To Port Yet

- WASM plugin install/run registry unless the target branch is ready for plugin
  execution.
- AssemblyScript compiler spike code.
- Mini Capsem file-plugin ABI tests, except as future plugin acceptance examples.

## Mainline Shape

The intended mainline dependency direction is:

```text
capsem-ui-catalog
  -> used by gateway/chat/MCP/UI endpoints
  -> used later by capsem-plugin-engine for ui.emit validation

capsem-plugin-engine
  -> depends on capsem-ui-catalog
  -> does not own UI catalogue or renderer contracts
```

During the transition, `capsem-plugin-engine` re-exports:

```rust
pub use capsem_ui_catalog::{ui, ui_tools};
```

That shim is only for compatibility with older prototype call sites. New mainline
code should import `capsem_ui_catalog` directly.

## Acceptance Gates

- `cargo test -p capsem-ui-catalog -- --nocapture`
- `cargo test -p capsem-a2ui-hyper -- --nocapture`
- frontend build/typecheck/tests for the renderer package in mainline
- browser verification that `/chat` or the equivalent main UI surface renders
  `ui.alert`, `ui.card`, `ui.table`, and `ui.ask`

## Risk To Watch

The catalogue tests currently look up `ui-preview/src/A2Node.svelte` from the
repo root to enforce renderer coverage. When porting into main Capsem, update
that path to the real Svelte renderer location rather than weakening the test.
