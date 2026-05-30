# UI Template Catalog Design

## Position

Capsem UI should emit A2UI v0.9 messages and render them through a Capsem
A2UI catalog. Preline is not part of the protocol. Preline is a trusted
template recipe layer used by the Svelte renderer.

The system has three separate artifacts:

1. **A2UI protocol schemas**: upstream message envelope and common types.
2. **Capsem catalog schema**: components, variants, enums, and properties that
   plugins/models are allowed to emit.
3. **Preline templates**: exact renderer recipes with machine-checkable
   binding annotations.

This keeps plugin output portable and auditable while still letting us render
with exact Preline component patterns.

## Flow

```text
context.ui.alert(...)
  -> UI MCP / structured authoring tools
  -> A2UI updateComponents message
  -> Capsem catalog JSON Schema validation
  -> Rust/TypeScript generated types
  -> template checker
  -> Svelte renderer
  -> Preline CSS recipe
```

Plugins never provide HTML, CSS, Tailwind class strings, Preline attributes, or
Svelte code. They can only emit objects accepted by the catalog schema.

## Authoring Surface

The model-facing authoring surface should look like structured tools, not a
template string. This mirrors how Codex works: the agent calls typed tools, the
host validates arguments, executes the operation, and returns structured
observations.

Minimum local UI tools:

```text
ui.catalog.list()
ui.catalog.describe(component)
ui.surface.create(id, kind, catalog)
ui.component.add(surface_id, component)
ui.surface.validate(surface_id)
ui.surface.preview(surface_id)
ui.surface.clear(surface_id)
```

Convenience tools such as `ui.alert`, `ui.button`, `ui.modal`, and `ui.card`
are allowed only if they lower to the same A2UI object and pass the same
catalog/template validators.

ArrowJS remains useful as an ergonomics reference: small vocabulary,
component-like composition, stable identity, and explicit bindings. It is not
the emitted object. Raw `html` templates, callbacks, DOM events, `.innerHTML`,
and arbitrary property bindings are not accepted from untrusted plugins/models.

## Acceptance Gate

The system must pass a self-use gate before we call it a plugin interface:

1. the user asks the agent to create a specific UI;
2. the agent builds it through local UI tools;
3. the tools emit validated A2UI messages;
4. the workbench renders through checked Preline templates;
5. validation and template reports are visible in the UI;
6. the agent can repair the surface from tool validation errors.

If this fails, the plugin interface is not real enough.

## Schema Source Of Truth

The source of truth is JSON Schema:

```text
schemas/a2ui/v0_9/server_to_client.json
schemas/a2ui/v0_9/common_types.json
schemas/capsem-ui/catalog.json
schemas/capsem-ui/templates/template.v1.schema.json
```

Rust and TypeScript types are generated from those schemas. Hand-written types
are acceptable only as temporary sprint scaffolding and must be tracked as debt.

## Template Shape

Promoted templates live under:

```text
templates/capsem-ui/<Component>/<variant>/
  template.html
  template.capui.json
  fixtures/
    default.a2ui.json
    expected.bindings.json
```

`template.html` uses exact Preline markup plus Capsem binding markers:

```html
<div class="..." role="alert">
  <span data-capui-text="message"></span>
</div>
```

`template.capui.json` names the component, variant, and bindings:

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

## Checker Rules

The template checker must prove:

- component exists in the selected catalog;
- variant is allowed by the catalog enum;
- binding props exist on the component schema;
- selector exists in the template;
- binding kind matches the prop schema;
- required renderable props are consumed or marked structural;
- unknown Capsem binding markers are rejected;
- Preline JS behavior markers are rejected or replaced by Svelte-owned state.

## Python Scrape Lane

Scraped Preline snippets are source material, not product artifacts:

```text
private/todo/preline/<component>/<name>/source.html
private/todo/preline/_manifest.json
```

The scraper should preserve exact snippets and make missing snippets explicit.
Reviewed snippets are promoted into tracked templates only after we annotate
and test their bindings.

## Workbench

The demo should become a real workbench:

- Preline sidebar for component/variant navigation;
- rendered preview first;
- inspector tabs for A2UI JSON, catalog slice, template source, template
  metadata, and checker report;
- validation status visible per example;
- all interactivity implemented in Svelte state.

This workbench is how we keep adding components without losing the plot.
