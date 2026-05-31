# Capsem Element Foundation Sprint

## Goal

Create a reusable Web Component foundation for generated UI islands. Charts,
rich blocks, and future plugin/model UI should share one host pattern instead
of each feature inventing its own Shadow DOM, lifecycle, and event bridge.

## Decisions

- The base custom element tag is `<capsem-elt>`.
- The TypeScript base class is `CapsemElement<TSpec>`.
- Shadow DOM is open by default so the host app can inspect and test output.
- Custom properties cross the Shadow DOM boundary, so Preline semantic tokens
  remain the theme source of truth.
- Component events leave the shadow boundary through composed `capsem:*`
  custom events.
- This is UI isolation, not a security boundary. Rust contracts and sandboxed
  plugin execution remain the security boundary.
- PM slide-deck requirement is now a design constraint on chart/export work:
  spreadsheets, charts, diagrams, images, text blocks, slides, and slide decks
  should share the same Rust-owned catalog path instead of becoming a separate
  presentation renderer later.
- Little Codex needs a per-instance SQLite workspace for manipulation: joins,
  grouping, filtering, pivots, and repeatable intermediate tables. This is a
  general Capsem data primitive, not just a slide/deck helper. It should feed
  typed sheets/tables/charts; it should not become a UI escape hatch.
- Since Capsem exposes its own local MCP, the first SQLite workbench surface
  should be MCP tools for agents. Plugin context methods can wrap the same Rust
  service later.
- `web.preview` needs its own later sprint as the browser-backed surface for
  showing user-visible pages, generated artifacts, and acceptance states.
- `generate.image`, `generate.video`, and `generate.audio` should be native
  media-generation APIs that create typed assets for UI, chat, slides, and deck
  export.

## Files

- `ui-preview/src/elements/capsem-elt.ts`
- `ui-preview/src/elements/index.ts`
- `ui-preview/src/main.ts`
- `test/capsem-element.test.ts`
- `CHANGELOG.md`

## Done

- `<capsem-elt>` is registered when the preview app starts.
- The base class owns Shadow DOM setup, spec updates, render scheduling,
  cleanup callbacks, and event emission.
- Tests guard the foundation pattern so specialized elements reuse it.
- Browser verification creates a `<capsem-elt>`, sets a spec, and observes
  shadow output plus composed event emission.

## Future Track

Capsem needs a typed spreadsheet-to-slide-deck lane:

- `context.data.sqlite` creates a constrained per-instance SQL workspace for
  data manipulation before rendering, reporting, telemetry inspection, or
  export.
- `capsem.data.sqlite.*` MCP tools are the first authoring lane for little
  Codex; they can feed `capsem.ui.*` render tools and prove the contract before
  WASM plugins are wired.
- `ui.sheet` creates spreadsheet-like data with named ranges.
- `ui.barChart`, `ui.lineChart`, `ui.heatmapChart`, and `ui.boxPlot` can bind to
  sheet ranges and produce stable export handles.
- `ui.diagram` starts with a Mermaid-backed variant while keeping the public API
  backend-neutral for later Graphviz or first-party diagram specs.
- `ui.slide` composes chart, diagram, image, text, table, and generated UI
  blocks.
- `ui.slideDeck` combines slides into an ordered deck artifact.
- `generate.image`, `generate.video`, and `generate.audio` create asset handles
  that can be embedded by UI blocks and exported into decks.
- `web.preview` shows rendered pages/artifacts to the user and supports later
  acceptance checks.
- Live preview can use Svelte/Preline/Web Components, but export must be
  deterministic from the Rust catalog spec.

## Proof Matrix

- Unit/contract: Vitest source contract for element foundation.
- Frontend: `npm run ui:build`, `npm run typecheck`, `npm test -- --run`.
- E2E/UI: browser DevTools verifies `<capsem-elt>` behavior.
- Adversarial: base element renders a typed error state for render failures.
- Telemetry: not applicable.
- Performance: not applicable.
