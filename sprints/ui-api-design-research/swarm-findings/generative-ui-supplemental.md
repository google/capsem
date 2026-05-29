# Generative UI Supplemental Findings

Status: completed
Agent: local
Scope: Follow-up research from user-provided sources after the initial swarm:
assistant-ui generative UI spec, A2UI, CopilotKit generative UI overview,
prompt-kit primitives, motion-primitives, and ArrowJS public docs.

## Sources Inspected

- `https://www.assistant-ui.com/docs/api-reference/generative-ui/spec`
- `https://a2ui.org/`
- `https://docs.copilotkit.ai/concepts/generative-ui-overview`
- `https://www.prompt-kit.com/primitives`
- `https://motion-primitives.com/`
- `https://arrow-js.com/`
- Local checkouts:
  - `private/upstream/A2UI` at `cba34e4`
  - `private/upstream/assistant-ui` at `9040b7c`
  - `private/upstream/prompt-kit` at `de80375`
  - `private/upstream/motion-primitives` at `92586e6`

## A2UI

A2UI is the strongest match for Capsem's model/plugin shared UI object. It is
a declarative protocol for rich UI across trust boundaries without executing
arbitrary code. The key concepts are:

- surfaces
- declarative component catalogs
- data binding
- flat adjacency-list component graphs
- streaming messages
- native/client-owned rendering

The important v0.9 message vocabulary is:

- `createSurface`
- `updateComponents`
- `updateDataModel`
- `deleteSurface`

The component graph is flat. Components reference children by ID instead of
nesting JSON trees. This matters because LLMs and streamed producers can add or
replace parts incrementally without regenerating a whole tree.

Capsem implication: use an A2UI-like catalog for the default structured block
path. Do not make arbitrary Arrow/Svelte/HTML the first answer for model or
plugin UI. The catalog is the security and design boundary.

## assistant-ui Generative UI

assistant-ui uses a message part:

```ts
{
  type: "generative-ui",
  spec: {
    root: GenerativeUINode | GenerativeUINode[],
  },
  id?: string,
  parentId?: string,
}
```

Each node is either a string or:

```ts
{
  component: string,
  props?: Record<string, unknown>,
  children?: GenerativeUINode[],
  key?: string,
}
```

The renderer resolves `component` against a consumer-provided allowlist. Unknown
components throw a typed error unless the host provides a fallback.

Capsem implication: this is an excellent small shape for chat-local generated
UI, but A2UI's flat adjacency-list model is better for streaming, patching, and
long-lived side panels.

## CopilotKit

CopilotKit frames generative UI as a spectrum:

- controlled: app-authored components/tools, agent supplies props or state
- declarative: agent emits a structured schema against a registered catalog
- open-ended: sandboxed UI supplied by MCP apps or external servers

Capsem implication: this is the right product taxonomy. Different surfaces can
choose different authority levels without creating different UI families.

- Most Capsem UI should be controlled or declarative.
- Open-ended renderer code is an escape hatch.
- MCP-hosted UI belongs in a sandboxed renderer lane, not in the trusted shell.

## prompt-kit

prompt-kit is useful as a chat block catalog reference, not as protocol. It has
good primitives for:

- message
- markdown
- tool
- reasoning
- source
- steps
- code block
- file upload
- prompt input
- system message

Capsem implication: these are good candidates for first-party Svelte block
types in `ui.chat(...)`.

## motion-primitives

motion-primitives is an implementation/design reference for polished
interactions. Components such as transition panels and text morphs are not
protocol primitives.

Capsem implication: motion belongs in trusted Svelte renderers for first-party
blocks. Plugins should not get arbitrary animation libraries or motion props as
part of the protocol surface.

## ArrowJS Public Docs

Arrow's public docs position it as tiny TypeScript primitives:

- `reactive`
- `html`
- `component`

The public sandbox model is explicit: JS/TS/Arrow logic runs inside a WASM VM,
while rendering can appear inline in the host app through a stable
`arrow-sandbox` custom element. The payload is virtual files: exactly one
`main.ts` or `main.js`, plus optional `main.css`. Sandbox code communicates to
the host through serialized `output(payload)`.

Capsem implication: Arrow remains a plausible custom-renderer spike. It is not
the default object model. The attractive part is model-friendly code and WASM
logic isolation; the risk is inline DOM authority. Capsem should only use it
after validating the DOM/property subset or keeping it behind a renderer frame.

## Updated Recommendation

Use three levels under one UI family:

1. Controlled built-in blocks, rendered by trusted Svelte.
2. Declarative catalog UI, A2UI-like, rendered by trusted Svelte/native clients.
3. Sandboxed custom renderers, ArrowJS/MCP-app-like, for cases the catalog
   cannot express.

The public API remains noun/surface-first:

```ts
ui.sidePanel("capsem.gitContext").replace(...);
ui.chat(threadId).append(...);
ui.modal("capsem.confirm").open(...);
ui.tab("capsem.trace").open(...);
```

Those calls can carry either first-party blocks, catalog component updates, or
a renderer reference. The surface name stays stable.
