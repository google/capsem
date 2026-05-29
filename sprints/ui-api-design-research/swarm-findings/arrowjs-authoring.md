# ArrowJS Authoring Findings

Status: completed
Agent: Gauss / `019e7493-35d7-7170-b62d-182d80e8281d`
Scope: Inspect local ArrowJS checkout and summarize author-facing API shape:
`html`, `reactive`, `component`, `watch`, event bindings, keyed templates,
unsafe/IDL surfaces, and whether its ergonomics are a good model for sandbox
renderers.

## Finding

Arrow's core surface is intentionally tiny: `html`, `svg`, `reactive`,
`watch`, `component`, `pick` / `props`, `nextTick`, and `onCleanup`.
Templates are tagged template literals or compiler-generated string arrays.
Templates are callable render objects and support identity helpers such as
`.key()` and `.id()`.

## Author-Facing Vocabulary

Arrow is strongest as a renderer authoring grammar:

```ts
const state = reactive({ count: 0 });

export default html`<button @click="${() => state.count++}">
  ${() => state.count}
</button>`;
```

The model-friendly pieces are:

- plain JS/TS state via `reactive`
- declarative markup via `html`
- function expressions for reactive reads
- `@click`-style event bindings
- dotted property binding for DOM properties
- keyed templates/components for identity

## What To Copy

Copy the ergonomics, not the raw trust model. Arrow shows that a compact
authoring layer can feel close to JavaScript without bringing a full app
framework into each plugin renderer.

`component()` is a useful pattern: stable local state per slot, live props, and
explicit callback channels. `watch()` is a useful pattern for scoped reactive
effects and cleanup.

## What To Reject Or Gate

Arrow parses static template strings through DOM template HTML. That means
model-authored static markup is trusted unless Capsem validates it first.

Gate or reject arbitrary IDL property bindings. `.innerHTML` works in Arrow and
therefore must not be accepted blindly in a Capsem sandbox renderer.

Do not expose raw DOM events to plugin code. Capsem should pass sanitized event
payloads.

## Capsem Implication

Arrow can inspire a renderer-level DSL for rich cards or panels, especially for
model-generated UI, but it should not define the top-level extension API.

The top-level API remains noun/surface based:

- `ui.sidePanel(...)`
- `ui.modal(...)`
- `ui.tab(...)`
- `ui.chat(...)`

Arrow-like `html` belongs inside declared renderers or trusted templates after
validation.

Transfer status: captured in sprint docs.
