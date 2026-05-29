# UI Catalog API Preview Sprint

## Goal

Build the minimal viable path from typed Capsem UI API to A2UI Basic format to
Svelte/Preline render preview.

This sprint proves the direction for the plugin system:

```text
plugin callback -> Rust UI API -> typed A2UI operation -> serialized format
  -> Svelte/Preline renderer
```

The plugin, MCP server, web server, validation, and format translation stay in
Rust. Svelte only consumes validated UI operations and paints Preline-shaped
components.

## Product Slice

Port A2UI v0.9 Basic as the baseline catalog contract. Do not invent a
Capsem-only component dictionary for this spike.

Render five Preline-shaped experiences by composing A2UI Basic components:

1. alert, composed from `Card`, `Row`, `Icon`, and `Text`
2. ask modal, composed from `Modal`, `Button`, `Column`, `Row`, and `Text`
3. card, composed from the upstream A2UI Basic `Card` component
4. status/badge-like callout, composed from `Card`, `Row`, `Icon`, and `Text`
5. chat baseline, composed from A2UI Basic components and chat ordering

The preview must show:

- the Rust API operations produced by `ui.alert(...)`, `ui.ask(...)`, and
  `ui.card(...)`;
- the A2UI Basic serialized format returned by the Rust server;
- the Svelte/Preline render output;
- an interactive ask modal with yes/no actions.

## Key Design Decisions

- Authoring is strong typed APIs and structs, not free-form JSON dictionaries.
- JSON is only the serialized transport format after Rust validation.
- A2UI v0.9 Basic is the baseline typed catalog with enum-backed component
  kinds.
- Chat UI is a surface/ordering convention over the same A2UI engine.
- Extension happens by adding catalog specs and typed constructors.
- Third-party/plugin authored UI will feed the same `UiOperation` path through
  a constrained context capability such as `context.ui.alert(...)` or
  `context.ui.sidePanel(...).replace(...)`.
- No arbitrary DOM, HTML, CSS, Preline JS, Tailwind class strings, or renderer
  code is accepted from plugins.

## Files To Create/Modify

- `crates/capsem-plugin-engine/src/lib.rs`
- `crates/capsem-plugin-engine/src/ui.rs`
- `crates/capsem-plugin-engine/tests/ui_catalog.rs`
- `crates/capsem-plugin-server/src/main.rs`
- `crates/capsem-plugin-server/src/ui_preview.rs`
- `crates/capsem-plugin-server/assets/ui-preview/*`
- `ui-preview/*`
- `package.json`
- `Cargo.toml`
- `docs/ui-api-authoring-synthesis.md`

## Implementation Order

1. Add Rust typed A2UI Basic models and API builders.
2. Add Rust validation and upstream A2UI fixture round-trip parsing.
3. Add server endpoint that produces demo UI operations from Rust API calls.
4. Add Svelte preview that fetches the endpoint and renders Preline-shaped UI.
5. Build preview assets and serve them from the Rust Axum server.
6. Verify Rust tests, Svelte build, and browser preview.

## Done Means

- Rust tests prove API builders create valid A2UI Basic operations.
- Rust tests prove serialized format parses back to strong typed operations.
- Invalid operations fail in Rust before Svelte sees them.
- Rust server exposes a preview page and JSON endpoint.
- Preview renders alert, ask modal, card, status callout, and chat baseline.
- Ask modal has yes/no controls in the UI.
- Sprint docs explain how plugin callbacks will feed this same path.

## Testing Proof Matrix

- Unit/contract: Rust tests for A2UI fixture round trips, builders, enum
  serialization, validation, and malformed payload rejection.
- Functional: HTTP endpoint returns typed demo operations and preview page.
- Adversarial: invalid catalog/component values reject in Rust.
- E2E/VM: Browser preview loads from the Rust server. No Capsem VM path in this
  isolated sprint.
- Telemetry: deferred; this sprint only exposes the operation shape that later
  telemetry will record.
- Performance: deferred; this sprint is API/format/render proof, not a
  throughput benchmark.
