# UI Extension Contract Research Sprint

## Goal

Investigate VS Code and Obsidian plugin UI contracts from primary sources so
Capsem's user-facing extension model is grounded in proven extension systems.

The core question is how Capsem should let plugins contribute rich chat output,
side panels, commands, and UI blocks while preserving isolation, reviewability,
and a JavaScript/Chrome-extension-like authoring model.

## Research Sources

- VS Code source and API definitions.
- VS Code extension samples when useful.
- Obsidian plugin API definitions and sample plugin.
- Obsidian developer docs when the closed-source app leaves API behavior
  implicit.

Fetched source must live under local ignored `private/` directories.

## Questions

- How do extensions declare UI contributions in manifests?
- How do they create side panels/views/webviews?
- How do they render rich interactive UI safely?
- How is message passing handled between extension code and UI surfaces?
- What APIs exist for commands, menus, status items, and workspace context?
- Where are isolation boundaries enforced?
- What should Capsem copy, avoid, or adapt?

## Deliverables

- Local ignored upstream source checkout under `private/upstream/`.
- A tracked findings document under `docs/`.
- Tracker updated with source versions and conclusions.

## Done Means

- Primary source files are identified and summarized.
- Findings compare VS Code and Obsidian side by side.
- Capsem design recommendations are concrete enough to drive the next prototype.
