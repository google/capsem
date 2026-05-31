# Sprint: Capsem Element Foundation

## Tasks

- [x] Write sprint plan and tracker.
- [x] Add reusable `CapsemElement<TSpec>` base class.
- [x] Register concrete `<capsem-elt>` foundation element.
- [x] Add contract tests.
- [x] Browser verify Shadow DOM and event emission.
- [x] Testing gate.
- [x] Changelog.
- [x] Commit.

## Notes

- User correction: chart should not get a one-off host; all generated UI
  islands need a common element foundation.
- Naming: keep future tool names consistent (`barChart`, `lineChart`,
  `heatmapChart`, `boxPlot`).
- `<capsem-elt>` is concrete and intentionally boring; specialized elements
  such as `capsem-chart` should subclass `CapsemElement<TSpec>`.
- Browser check created a `capsem-elt`, assigned a typed spec, observed an open
  ShadowRoot, and received a composed/bubbling `capsem:ready` event with the
  spec id.
- PM slide-deck track captured: spreadsheet data -> chart, diagram, image,
  text, and table slide blocks -> ordered slide deck. Chart and diagram work
  now needs export handles and sheet-range bindings, not just live preview
  rendering.
- Data manipulation note captured: add a constrained SQLite scratch workbench
  for little Codex so report/chart/deck data can be joined, filtered, grouped,
  and reused without stuffing raw arrays into prompts.
- Local MCP note captured: expose the SQLite workbench first through Capsem MCP
  tools, then wrap the same Rust service in plugin context APIs later.

## Coverage Ledger

- Unit/contract: `npm test -- --run test/capsem-element.test.ts test/ui-semantic-tokens.test.ts` passes.
- Frontend: `npm run ui:build`, `npm run typecheck`, and `npm test -- --run` pass.
- E2E/UI: Chrome DevTools verified `<capsem-elt>` registration, Shadow DOM,
  typed spec rendering, and composed event emission.
- Adversarial: base class renders an error part and emits `capsem:error` on
  render failures.
- Telemetry: not applicable.
- Performance: not applicable.
- Missing/deferred: specialized chart element and Plotly adapter are next sprint.
- Missing/deferred: spreadsheet, diagram, slide, and slide-deck catalog types
  are a future export/product track, but the current chart design must preserve
  that path.
- Missing/deferred: SQLite scratch database API, query limits, type mapping,
  import/export boundaries, and audit logging need their own data sprint before
  finance/science report workflows become real.
