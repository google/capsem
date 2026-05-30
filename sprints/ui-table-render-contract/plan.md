# UI Table Render Contract Sprint

## Goal

Make `ui.table` a typed Rust UI tool that lowers to conformant A2UI Basic and renders as a real Preline table in the clean chat shell.

The key correction is the contract gate: if Rust exposes a typed UI recipe, the Svelte renderer must have an explicit recipe branch for it. A typed API without a renderer is drift and should fail tests.

## Decisions

- Keep A2UI Basic as the wire format.
- Use recipe metadata only to select the Preline adapter, not to smuggle HTML or CSS.
- Add a general Rust test that scans `ui-preview/src/A2Node.svelte` and checks every registered typed recipe component has an explicit renderer branch.
- Add `ui.table` only as a typed tool. Raw `ui.component.add` remains possible for low-level experiments, but it is not acceptable for polished product output.

## Files

- `crates/capsem-plugin-engine/src/ui_tools.rs`
- `crates/capsem-plugin-engine/tests/ui_tools.rs`
- `crates/capsem-a2ui-hyper/tests/a2ui_http.rs`
- `ui-preview/src/A2Node.svelte`
- `CHANGELOG.md`

## Done

- `ui.table` accepts columns and rows and rejects malformed tables.
- Rust and Hyper contract tests prove the output is A2UI conformant.
- Renderer-drift test fails if a typed recipe component lacks an explicit Svelte branch.
- `/chat` can show the Game of Thrones table through a live tool call.
- Focused Rust and frontend gates pass.

## Proof Matrix

- Unit/contract: `capsem-plugin-engine` tests for `ui.table`, renderer coverage, and malformed input.
- Functional: Hyper E2E `POST /a2ui/render` table test.
- Adversarial: reject empty columns and rows with wrong cell counts.
- E2E/UI: live `/ui/tools/run` table call rendered on `/chat`, browser-checked for an actual `<table>`.
- Telemetry: deferred; UI tool telemetry is not wired in this isolated sprint.
- Performance: deferred; table rendering is not performance-critical for this correction.
