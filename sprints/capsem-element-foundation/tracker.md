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
- PM slide-deck track captured: spreadsheet data -> chart/image/text/table
  slide blocks -> ordered slide deck. Chart work now needs export handles and
  sheet-range bindings, not just live preview rendering.

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
- Missing/deferred: spreadsheet, slide, and slide-deck catalog types are a
  future export/product track, but the current chart design must preserve that
  path.
