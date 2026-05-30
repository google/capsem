# A2UI Preline Template System Sprint

## Goal

Turn the UI preview spike into a disciplined, machine-checkable UI system:

```text
plugin/model/Rust API
  -> A2UI v0.9 message envelope
  -> Capsem A2UI catalog JSON Schema
  -> generated Rust + TypeScript types
  -> checked Preline template recipes
  -> Svelte-only renderer workbench
```

The hard rule: plugins and models emit typed A2UI messages. They do not emit
HTML, Preline class strings, Tailwind classes, Svelte code, or renderer code.
Preline is a trusted renderer template layer that we can scrape, inspect,
annotate, and machine-check against the schema.

## Product Slice

Build a useful local workbench where we can inspect and prove the full path:

1. exact Preline snippets scraped into ignored `private/todo/preline/...`;
2. reviewed templates promoted into tracked `templates/capsem-ui/...`;
3. Capsem A2UI catalog schemas define the component surface;
4. a checker proves template bindings match catalog properties;
5. generated Rust and TypeScript types drive the demo and Svelte renderer;
6. the preview page has a real Preline layout with a left sidebar, component
   catalogue navigation, rendered preview, A2UI JSON, template binding report,
   and source/template panes.

This sprint should make the demo pleasant enough to use while we keep adding
components. The page is a workbench, not a landing page.

## Key Design Decisions

- A2UI v0.9 is the wire protocol. We vendor or pin the upstream schemas:
  `server_to_client.json`, `common_types.json`, and client/action schemas as
  needed.
- Capsem additions are expressed as an A2UI-compatible catalog JSON Schema,
  not a second protocol.
- JSON Schema is the source of truth for generated Rust and TypeScript types.
  Rust structs/enums and frontend types are artifacts.
- Component kinds, variants, tones, button styles, modal recipes, and action
  shapes are JSON Schema `const`, `enum`, discriminators, and refs. No custom
  mini type language.
- A2UI component properties are the API fields. We do not duplicate them into a
  separate `fields` list plus `bindings` list.
- Preline templates are trusted renderer assets with Capsem binding markers.
  They are not protocol objects, but they are not loose HTML either.
- Template metadata exists only to verify binding correctness and trace the
  recipe origin. It is not emitted by plugins.
- Svelte owns all interactivity. We copy Preline CSS class recipes, but we do
  not use Preline JS, `data-hs-*` behavior, or plugin-authored DOM.
- The UI workbench uses Svelte components only. No React, no arbitrary HTML
  injection, no runtime template execution from untrusted input.

## Template Contract

Each promoted template lives under:

```text
templates/capsem-ui/<Component>/<variant>/
  template.html
  template.capui.json
  fixtures/
    default.a2ui.json
    expected.bindings.json
```

The HTML file is exact Preline structure plus binding markers such as:

```html
<div class="..." role="alert">
  <span data-capui-text="message"></span>
</div>
```

The metadata file is deliberately small:

```json
{
  "schema": "capsem.ui-template.v1",
  "component": "Alert",
  "variant": "soft",
  "bindings": [
    {
      "prop": "message",
      "kind": "text",
      "selector": "[data-capui-text='message']"
    }
  ]
}
```

The checker must prove:

- the component exists in the selected catalog;
- the variant is allowed by the catalog enum;
- every binding references an existing property;
- every selector exists in the template;
- the binding kind matches the property schema shape;
- every required renderable property is either bound or explicitly structural;
- no unknown Capsem binding marker appears in the template;
- Preline JS behavior markers are rejected or translated into Svelte-owned
  state.

## Python Scraper

Add a Python scraper that extracts exact Preline snippets into ignored private
files:

```text
private/todo/preline/alert/<name>/source.html
private/todo/preline/modal/<name>/source.html
private/todo/preline/card/<name>/source.html
private/todo/preline/_manifest.json
```

The scraper should:

- use stdlib first: `urllib.request`, `html.parser` or carefully bounded
  extraction, `html.unescape`, and JSON output;
- find docs snippets by textarea id or marker text;
- write exact source snippets without reformatting;
- write manifest entries with component, variant candidate, source URL,
  selector/textarea id, output path, and extraction timestamp;
- never commit the scraped HTML from `private/`;
- make missing snippets explicit instead of inventing replacements.

## Schema And Generation

Add tracked schemas under:

```text
schemas/a2ui/v0_9/
schemas/capsem-ui/
schemas/capsem-ui/templates/
```

Generation target:

- Rust: protocol/catalog structs and enums used by the Rust server and tests;
- TypeScript: Svelte renderer/workbench types;
- JSON Schema fixtures: validation fixtures for positive and negative cases.

If the generator choice is not settled in this sprint, the fallback is to
vendor schemas and hand-write a narrow Rust/TS model only as a temporary
fixture, with an explicit tracker debt item.

## UI Workbench

Replace the cramped preview with a useful workbench:

- left sidebar with component groups and variants;
- main rendered preview at the top;
- inspector tabs for A2UI message, catalog schema slice, template metadata,
  template source, and checker report;
- visible pass/fail status for schema validation and template binding checks;
- modal/alert/card/button examples rendered with Preline classes;
- no explanatory marketing copy in the app surface;
- responsive layout that remains usable on narrow browser windows.

The workbench should be attractive enough to stare at during design review:
Preline tokens, real spacing, restrained card boundaries, no nested card soup,
no generic grey boxes, and no raw Tailwind color hacks.

## Files To Create/Modify

- `scripts/extract_preline_blocks.py`
- `schemas/a2ui/v0_9/*`
- `schemas/capsem-ui/catalog.json`
- `schemas/capsem-ui/templates/template.v1.schema.json`
- `templates/capsem-ui/*`
- `crates/capsem-plugin-engine/src/ui.rs`
- `crates/capsem-plugin-engine/tests/ui_templates.rs`
- `crates/capsem-plugin-server/src/ui_preview.rs`
- `ui-preview/src/*`
- `docs/ui-template-catalog-design.md`
- `docs/ui-fixme.md`
- `sprints/a2ui-preline-template-system/tracker.md`
- `CHANGELOG.md`

## Implementation Order

1. Vendor/pin upstream A2UI v0.9 schema files and record source/version.
2. Add `scripts/extract_preline_blocks.py` and scrape initial Preline alert,
   modal, card, button, and badge/card-status source snippets into
   `private/todo/preline`.
3. Define the first Capsem catalog slice as A2UI-compatible JSON Schema.
4. Define `template.capui.json` schema and one promoted template.
5. Add template checker tests against the catalog and template metadata.
6. Wire Rust preview endpoint to serve examples plus validation/check reports.
7. Rebuild Svelte workbench with Preline sidebar, preview, and inspector panes.
8. Promote the remaining initial templates and make failures visible.
9. Run Rust tests, frontend build/tests, and browser verification.

## Done Means

- We can scrape exact Preline source into ignored private files.
- At least five reviewed templates are tracked with binding metadata:
  alert, modal, card, button, and a status/badge/card-style block.
- The checker rejects bad props, bad variants, missing required bindings,
  unknown markers, and selectors that do not exist.
- The demo emits A2UI messages and validates them against the selected catalog.
- The Svelte workbench renders the checked templates and shows the validation
  evidence without relying on plugin-authored HTML.
- The browser preview is usable for continued component review.
- Sprint docs capture the remaining generator/dependency decision if not fully
  implemented.

## Testing Proof Matrix

- Unit/contract: JSON Schema validation for A2UI messages, Capsem catalog
  components, and template metadata; Rust tests for checker behavior.
- Functional: server endpoint returns examples, validation reports, and
  template check reports consumed by the Svelte workbench.
- Adversarial: malformed component kind, unknown variant, unknown binding prop,
  missing selector, missing required renderable prop, and Preline JS behavior
  markers are rejected.
- E2E/VM: in-app browser loads the workbench from the Rust server and exercises
  sidebar navigation plus modal interaction. No Capsem VM integration in this
  isolated sprint.
- Telemetry: deferred; report objects are shaped so later plugin telemetry can
  record render/check results.
- Performance: deferred except for keeping template checks cheap enough for
  load-time install validation.

## Release Holds

- Do not wire this into production Capsem until schema generation is settled.
- Do not accept plugin-authored HTML/classes.
- Do not rely on Preline JS behavior.
- Do not claim full A2UI Basic support until upstream fixture coverage is in
  the test suite.
