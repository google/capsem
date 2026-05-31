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
- Data manipulation note captured: add a constrained per-instance SQLite
  workspace for little Codex so data can be joined, filtered, grouped, and
  reused without stuffing raw arrays into prompts. This is general, not only for
  report/chart/deck workflows.
- Local MCP note captured: expose the SQLite workbench first through Capsem MCP
  tools, then wrap the same Rust service in plugin context APIs later.
- Web preview note captured: add a later `web.preview` sprint for user-visible
  browser-backed previews and acceptance surfaces.
- Media generation note captured: add `generate.image`, `generate.video`, and
  `generate.audio` as typed asset-producing APIs, not raw blob escapes.
- Gemini spike note captured: implement `generate.*` first through Capsem MCP
  backed by the existing Gemini API key, with explicit config errors when the
  key is missing.
- Multimodal generation note captured: image/video/audio generation should
  accept typed asset inputs such as reference images, masks, clips, voice
  samples, and timing/storyboard hints.
- MCP grouping note captured: group tools by product noun as the surface grows:
  data, ui, generate, asset, export, and web preview.

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
- Missing/deferred: per-instance SQLite API, query limits, type mapping,
  import/export boundaries, lifetime policy, and audit logging need their own
  data sprint before finance/science/report workflows become real.
- Missing/deferred: `web.preview` surface contract, preview lifecycle,
  screenshot/inspection semantics, and acceptance-check routing need a browser
  surface sprint.
- Missing/deferred: generated image/video/audio asset schemas, provider
  routing, storage, permissions, and export behavior need a media generation
  sprint.
- Missing/deferred: provider configuration UI/policy for Gemini, OpenAI, local,
  and future providers is intentionally out of the first Gemini-backed spike.
