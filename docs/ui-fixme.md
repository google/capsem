# UI FIXME

This document tracks UI mistakes and design gaps found while building the
A2UI-to-Preline preview. Treat it as the board for the next UI sprint.

## Current Direction

Capsem should use Preline as the component recipe source of truth.

The intended path is:

```text
Typed Capsem UI API -> A2UI Basic object graph -> renderer adapter
  -> exact Preline component recipe classes with A2UI fields inserted in slots
```

The renderer can own behavior in Svelte, but it should not invent a parallel
visual system when the component claims to be Preline.

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

A2UI Basic gives us portable primitives:

- `Card`
- `Modal`
- `Button`
- `Text`
- `Icon`
- `Row`
- `Column`
- `List`
- form controls and media primitives

It does not appear to encode every Preline visual variant directly. That is not
automatically a blocker, but it means Capsem needs a typed authoring layer that
knows:

- the desired Preline recipe;
- the recipe variant;
- which A2UI child maps to which Preline slot;
- which choices are lost when serializing through Basic.

If A2UI Basic cannot carry a variant, we must either:

- add a Capsem-side catalogue convention around Basic composition; or
- define a richer Capsem UI catalogue that still renders through the same safe
  renderer engine.

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

## Immediate Next Steps

1. Replace remaining local approximations in the preview renderer with exact
   Preline docs classes for supported recipes.
2. Add typed Rust enums for alert, card, modal, badge, and button variants.
3. Move preview-only recipe metadata into the real Rust authoring API.
4. Add tests that snapshot the renderer adapter contract at the data/slot level.
5. Decide whether variants live as Capsem metadata around A2UI Basic or require
   a richer Capsem UI catalogue.
