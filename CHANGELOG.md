# Changelog

## [Unreleased]

### Added

- Added a Rust-owned A2UI v0.9 Basic preview path with typed UI helpers,
  contract tests, server endpoints, and a Svelte/Preline renderer preview.
- Added an A2UI/Capsem/Preline template-system sprint and design note covering
  JSON Schema source of truth, Python Preline scraping, checked template
  bindings, and a Svelte workbench.
- Added the UI MCP-style authoring acceptance gate: the agent must build a
  requested UI through structured tools and render it in the workbench.

### Changed

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
