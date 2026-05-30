# UI FIXME

This document tracks UI mistakes and design gaps found while building the
A2UI-to-Preline preview. Treat it as the board for the next UI sprint.

## Current Direction

Capsem UI should emit A2UI v0.9 messages and validate them against a Capsem
A2UI catalog JSON Schema. Preline is not the protocol. Preline is the trusted
Svelte renderer template layer.

The intended path is:

```text
Typed Capsem UI API -> A2UI v0.9 message -> Capsem catalog validation
  -> checked Preline template -> Svelte renderer
```

The renderer owns behavior in Svelte, but exact Preline recipes must be tracked
as templates with machine-checkable bindings.

## Errors To Fix

- The preview must distinguish Preline v4 semantic classes that appear in the
  docs (`bg-card`, `text-foreground`, `border-card-line`) from Capsem-invented
  approximations. Preline's exact recipe strings are allowed; local
  translation is not.
- Earlier commits treated Preline as a theme/token provider instead of as a set
  of documented component recipes. That made the preview look Preline-ish
  rather than Preline-correct.
- Some renderer branches use generic recursive rendering for `Row`, `Column`,
  and fallback `Card` shapes. That is useful for debugging A2UI, but it is not
  a production component adapter.
- The current `Ui::alert(msg)` API erases the Preline alert variant. Preline has
  solid, soft, bordered, dismissible, icon, action, and contextual variants. The
  API exposes none of that.
- The current `Ui::card(title, description)` API exposes only a thin simple-card
  path. Preline cards have body-only, header/footer, image, list-group, link,
  horizontal, sizing, and grid compositions. We have no typed way to select
  those.
- The current `Ui::ask(text, yes, no)` API exposes a modal intent, but not the
  Preline modal size, scroll behavior, header/body/footer slot structure, or
  action button style.
- The current status callout is an invented convenience over `Card`, `Row`,
  `Icon`, and `Text`. It may be acceptable as a Capsem helper, but it must map
  to a named Preline recipe or be renamed as a Capsem composition.
- The preview now has per-example recipe metadata, but the production API still
  does not. Component ids should stop doing semantic work in the renderer.
  The API should carry recipe/variant/slot intent explicitly when A2UI Basic
  allows it, or the helper should generate a stable wrapper convention
  documented as part of Capsem's UI catalogue.

## A2UI Constraint Check

A2UI gives us a portable envelope and basic primitives:

- `Card`
- `Modal`
- `Button`
- `Text`
- `Icon`
- `Row`
- `Column`
- `List`
- form controls and media primitives

It does not encode every Preline visual variant directly. That is not a
blocker, but it means Capsem needs an A2UI-compatible catalog schema and a
checked template layer that knows:

- the component and variant enum;
- which A2UI child maps to which Preline slot;
- which properties are structural versus directly rendered;
- which exact Preline template implements the component.

## API Shape We Should Move Toward

The authoring API should expose Preline choices instead of making the renderer
infer them.

Examples:

```text
ui.alert(msg="...", variant=AlertVariant::Soft, tone=Tone::Info)
ui.alert(msg="...", variant=AlertVariant::Bordered, dismissible=true)
ui.card(title="...", body="...", variant=CardVariant::Simple)
ui.card(...).header(...).body(...).footer(...)
ui.ask(text="...", yes="...", no="...", size=ModalSize::Md)
```

This stays strongly typed. No plugin-authored HTML, CSS, Tailwind classes, or
Preline JS enters the renderer.

## Renderer Rule

For each supported component, keep a fixture that contains:

- the Rust API call;
- the serialized A2UI messages;
- the Preline docs recipe we claim to implement;
- the exact class recipe used by the Svelte renderer;
- the slot mapping from A2UI fields/children to Preline placeholders.

The renderer should fail closed for unsupported recipe variants instead of
falling back to a fake generic box.

## Fixed In The Isolated Prototype

1. Upstream A2UI v0.9 schemas are pinned under `schemas/a2ui/v0_9`.
2. Exact Preline docs snippets are scraped into ignored
   `private/todo/preline` when the docs expose extractable examples.
3. The first Capsem catalog schema slice and template metadata schema exist
   under `schemas/capsem-ui`.
4. Five reviewed templates are tracked under `templates/capsem-ui`.
5. The Svelte workbench has a sidebar, rendered preview first, and A2UI/tool
   inspector panes.
6. The self-use gate builds the first rendered surface through structured UI
   tools, not hand-authored JSON.

## Next Fixes

1. Replace the temporary Rust contract map in the template checker with code
   generated from JSON Schema.
2. Expand the Preline scraper so it can recover modal and card variants that
   are not exposed through the first textarea extractor.
3. Make template reports visible per promoted template, not only through the
   Rust tests and workbench metadata pane.
4. Remove generic fallback rendering from production lanes once every supported
   component has a checked adapter.
5. Decide the exact Capsem catalog extension path for Preline variants that
   A2UI Basic does not model directly.
