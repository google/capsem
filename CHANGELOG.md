# Changelog

## [Unreleased]

### Added

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
- Added a Rust-owned A2UI v0.9 Basic preview path with typed UI helpers,
  contract tests, server endpoints, and a Svelte/Preline renderer preview.
- Added an A2UI/Capsem/Preline template-system sprint and design note covering
  JSON Schema source of truth, Python Preline scraping, checked template
  bindings, and a Svelte workbench.
- Added the UI MCP-style authoring acceptance gate: the agent must build a
  requested UI through structured tools and render it in the workbench.

### Changed

- Removed the internal `A2UI Basic component: Card` caption from user-facing
  card output so protocol labels do not bleed into generated UI.
- Replaced remaining raw palette utilities in the generated UI renderer path
  with Preline semantic token classes.
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
