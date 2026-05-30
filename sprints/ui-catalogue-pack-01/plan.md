# UI Catalogue Pack 01 Sprint

## Goal

Ship the first useful UI block pack inside `capsem-ui-catalog`:

- `ui.notice`
- `ui.card` with image and actions
- `ui.facts`
- `ui.table` with search, filtering, and pagination metadata
- `ui.ask` with typed choices

## Decisions

- Keep the wire format A2UI Basic.
- Use recipe metadata only for renderer adapter options, such as table controls.
- Lower images/actions/choices/facts into A2UI Basic components, not raw HTML.
- Render all new blocks with Preline semantic tokens in `A2Node.svelte`.
- Preserve existing `ui.card`, `ui.table`, and `ui.ask` calls for compatibility.

## Files

- `crates/capsem-ui-catalog/src/ui_tools.rs`
- `crates/capsem-ui-catalog/tests/ui_tools.rs`
- `crates/capsem-a2ui-hyper/tests/a2ui_http.rs`
- `ui-preview/src/a2ui.ts`
- `ui-preview/src/A2Node.svelte`
- `test/ui-semantic-tokens.test.ts`
- `CHANGELOG.md`

## Done

- All five tools lower to conformant A2UI Basic.
- Renderer-drift gate covers all five recipe components.
- `/chat` can render the pack through `POST /ui/tools/run`.
- Table search/filter/page controls work in the browser.
- Card image renders without raw HTML or raw CSS input.

## Proof Matrix

- Unit/contract: `cargo test -p capsem-ui-catalog`.
- Integration: `cargo test -p capsem-a2ui-hyper`.
- Frontend: `npm run ui:build`, `npm run typecheck`, `npm test -- --run`.
- Adversarial: malformed table rows still reject; raw renderer input still rejects.
- E2E/UI: browser verifies `/chat` renders the pack and table controls update row counts.
- Telemetry: not applicable.
- Performance: not applicable for this pack.
