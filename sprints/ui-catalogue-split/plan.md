# UI Catalogue Split Sprint

## Goal

Split the A2UI/Capsem UI catalogue work out of the WASM plugin engine so it can land independently and be reused by chat, MCP, gateway, plugin callbacks, and future model-generated UI.

## Decisions

- Create a standalone Rust crate: `capsem-ui-catalog`.
- Move A2UI Basic message types, typed `Ui` helpers, UI tool runner, recipe metadata, and template checker into that crate.
- Keep `capsem-plugin-engine` focused on plugin install/run/ABI work.
- Keep compatibility re-exports from `capsem-plugin-engine` for now, but update server, Hyper, and catalogue tests to depend on `capsem-ui-catalog` directly.
- Preserve the existing Svelte renderer path; the split is crate ownership, not a UI rewrite.

## Files

- `Cargo.toml`
- `crates/capsem-ui-catalog/**`
- `crates/capsem-plugin-engine/Cargo.toml`
- `crates/capsem-plugin-engine/src/lib.rs`
- `crates/capsem-a2ui-hyper/Cargo.toml`
- `crates/capsem-a2ui-hyper/src/lib.rs`
- `crates/capsem-a2ui-hyper/tests/a2ui_http.rs`
- `crates/capsem-plugin-server/Cargo.toml`
- `crates/capsem-plugin-server/src/**`
- `CHANGELOG.md`

## Done

- UI catalogue crate builds and owns its tests.
- Plugin engine has no local `ui.rs` or `ui_tools.rs` implementation.
- Hyper/server import catalogue APIs directly.
- Existing plugin tests still pass.
- Existing UI preview and `/chat` behavior still works.

## Proof Matrix

- Unit/contract: `cargo test -p capsem-ui-catalog`.
- Integration: `cargo test -p capsem-a2ui-hyper`.
- Regression: `cargo test -p capsem-plugin-engine`.
- Frontend: existing semantic-token and build checks remain unchanged.
- E2E/UI: restart server and verify `/chat` still renders authored table.
- Telemetry: not applicable to this split.
- Performance: not applicable; crate boundary only.
