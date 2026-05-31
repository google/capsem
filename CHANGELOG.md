# Changelog

## [Unreleased]

### Added

- Added a reusable `<capsem-elt>` Web Component foundation with Shadow DOM,
  typed spec assignment, lifecycle cleanup, and composed Capsem event emission.
- Documented the future spreadsheet-to-slide-deck UI track so chart, diagram,
  and export work preserves sheet ranges, export handles, slide blocks, and
  deck composition.
- Documented the future SQLite scratch data workbench requirement for
  model-assisted data manipulation before rendering sheets, charts, and decks.
- Clarified that the SQLite data workbench should first surface through
  Capsem's local MCP tools so agents can use it before the WASM plugin runtime.
- Added the isolated UI tool workbench lane: pinned A2UI schemas, Capsem UI
  catalog/template schemas, checked Preline templates, Rust UI tool runner
  endpoints, and a Svelte workbench that renders the tool-built acceptance
  surface first.
- Added a live UI authoring lane so `POST /ui/tools/run` stores the latest
  structured tool result, `/ui/spec/demo` exposes it, and the workbench renders
  the authored "Agent Draft" first.
- Added a chat-shell workbench view that renders the authored A2UI component
  inside an assistant message while preserving the inspector panes.
- Added a clean `/chat` page for the user-facing chat shell, separate from the
  diagnostic workbench.
- Added a Hyper-only A2UI contract lane with E2E conformance tests and a tested
  TypeScript response decoder.
- Added contract support for alert tone metadata and card link metadata so
  accepted `ui.alert` and `ui.card` fields survive to rendering.
- Added `ui.table` as a typed Rust UI tool with A2UI conformance coverage and
  an explicit Preline table renderer.
- Added a renderer-drift contract test so typed Rust UI recipe components fail
  if the Svelte renderer has no explicit adapter.
- Added `/chat` theme and dark-mode controls backed by real Preline packaged
  themes for semantic-token visual checks.
- Added a semantic-token regression test for the generated UI renderer path.
- Added a standalone `capsem-ui-catalog` crate so the A2UI/Preline catalogue can
  land independently of the full WASM plugin runtime.
- Added UI Catalogue Pack 01: `ui.notice`, image/action cards, `ui.facts`,
  searchable/filterable/paginated tables, and choice-based `ui.ask`.
- Added a Rust-owned A2UI v0.9 Basic preview path with typed UI helpers,
  contract tests, server endpoints, and a Svelte/Preline renderer preview.
- Added an A2UI/Capsem/Preline template-system sprint and design note covering
  JSON Schema source of truth, Python Preline scraping, checked template
  bindings, and a Svelte workbench.
- Added the UI MCP-style authoring acceptance gate: the agent must build a
  requested UI through structured tools and render it in the workbench.

### Changed

- Changed `ui.ask` from a modal launcher to an inline decision card with a
  title, optional detail text, and two visible action buttons.
- Updated the A2UI modal renderer with Svelte dialog behavior: focus restore,
  Escape/backdrop close, scroll lock, and keyboard focus cycling.
- Removed the internal `A2UI Basic component: Card` caption from user-facing
  card output so protocol labels do not bleed into generated UI.
- Fixed generated button labels so nested A2UI text inherits the button
  foreground color instead of rendering muted grey on primary buttons.
- Replaced remaining raw palette utilities in the generated UI renderer path
  with Preline semantic token classes.
- Moved A2UI message types, typed UI helpers, UI tools, and template checks out
  of `capsem-plugin-engine` ownership and into `capsem-ui-catalog`.
- Updated `/chat` to render every authored UI surface in a tool result so block
  packs can be previewed together.
- Updated the UI preview to show the actual A2UI Basic `Card` component with
  Preline card tokens, and cross-check Rust messages with `a2ui-types`.
- Moved the rendered preview above the A2UI JSON and restyled the shell around
  Preline's docs/card theme.
- Replaced the preview's Tailwind CDN/token mimic with the real `preline`
  package processed through the Tailwind v4 Vite pipeline.
- Remapped the UI preview renderer from generic A2UI tree dumping to
  Preline recipe adapters for alerts, cards, card-style tabs, buttons, badges,
  and modal shells.
- Added explicit Preline recipe metadata to UI preview examples so the renderer
  maps A2UI fields into named component variants instead of guessing from ids.
