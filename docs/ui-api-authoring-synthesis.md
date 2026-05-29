# UI API Authoring Synthesis

## Correction

`emit` is not an authoring API. It is an internal lowering/transport operation.

The public API must be made of named UI surfaces:

```ts
ui.sidePanel(...);
ui.modal(...);
ui.tab(...);
ui.chat(...);
ui.statusItem(...);
ui.renderer(...);
```

Authors should think like Chrome extension authors: declare surfaces in a
manifest, then call focused methods on those surfaces. They should not think in
host messages, sockets, iframes, or patch routing.

## Research Synthesis

Chrome teaches the strongest API shape: static manifest declarations plus
runtime noun APIs. Copy `sidePanel`, `commands`, `contextMenus`, `action`,
permissions, and option-object methods. Do not copy browser-specific
`tabs/windows` semantics unless Capsem has a real corresponding workspace
surface.

Agent UI protocols teach the data model: split model-visible content from
renderer-private content, use stable IDs, snapshots/deltas, and message parts.
OpenAI Apps SDK is the best privacy model. Vercel AI SDK is the best chat
transcript shape. AG-UI is the best live-update vocabulary.

A2UI is the strongest default shape for generated UI across trust boundaries:
declarative surfaces, component catalogs, data binding, flat adjacency-list
component graphs, and streaming updates without arbitrary code execution. This
should influence Capsem's structured block/catalog path more than any renderer
library should.

assistant-ui's generative UI spec is the simplest useful reference: a
`generative-ui` message part carries a JSON component tree, and the renderer
resolves names against a consumer-provided allowlist. It is good for chat-local
component trees. A2UI's flat model is better for long-lived panels and patches.

CopilotKit gives the product taxonomy: controlled components, declarative
catalog UI, and open-ended sandboxed UI. Capsem should support all three under
one UI family, but the default should be controlled/declarative.

ArrowJS teaches renderer ergonomics: plain JS state, tagged template rendering,
small runtime, and model-friendly syntax. Copy that feel for sandbox renderers,
but validate markup and gate unsafe property bindings such as `.innerHTML`.

prompt-kit and motion-primitives are design/implementation references for
first-party block catalogs and polished interactions. They are not protocol
surfaces.

Capsem's frontend teaches placement ownership: Svelte owns the shell, tabs,
toolbars, side panels, modals, and styling. Plugins provide structured blocks,
commands, state, and optional sandbox renderer entries.

## Manifest Shape

```ts
export default Plugin({
  id: "capsem.git-context",
  name: "Git Context",
  permissions: ["fs.read", "https.fetch", "ui.sidePanel"],
  contributes: {
    commands: [
      { id: "capsem.gitContext.reveal", title: "Show Git Context" },
    ],
    ui: {
      sidePanels: [
        {
          id: "capsem.gitContext",
          title: "Git Context",
          scope: "workspace",
          placement: "right",
        },
      ],
      tabs: [],
      modals: [],
      statusItems: [],
      renderers: [],
    },
  },
});
```

## Runtime Authoring Shape

```ts
Plugin("capsem.git-context")
  .onFileCreate(async (file, context) => {
    if (file.path !== ".git" && !file.path.endsWith("/.git")) {
      return file;
    }

    const head = await context.fs.read(`${file.path}/HEAD`);
    const branch = head.startsWith("ref: refs/heads/")
      ? head.slice("ref: refs/heads/".length).trim()
      : "detached";

    await context.ui.sidePanel("capsem.gitContext").replace({
      scope: { workspaceId: context.workspace.id },
      title: "Git Context",
      blocks: [
        {
          kind: "keyValue",
          id: `git:${context.workspace.id}`,
          rows: [
            ["project", context.workspace.name],
            ["branch", branch],
            ["worktree", context.workspace.root],
          ],
        },
      ],
    });

    return file;
  });
```

The plugin callback still returns the mutated object it received. UI work is a
side effect through a constrained context capability, but the authoring object
is `ui.sidePanel(...)`, not `emit`.

## Lowered Host Object

Surface calls compile into an internal object like this:

```ts
{
  kind: "ui.surface",
  action: "replace",
  surface: "sidePanel",
  id: "capsem.gitContext",
  scope: { workspaceId: "..." },
  title: "Git Context",
  modelContent: { summary: "Git branch main" },
  privateContent: { rawHead: "ref: refs/heads/main" },
  update: {
    mode: "snapshot",
    blocks: [],
  },
}
```

This object is for validation, logging, remote UI routing, and renderer
dispatch. It is not the API we teach plugin authors.

## Surface Semantics

`ui.sidePanel(id)` is for durable contextual workspace or VM panels. It supports
`open`, `reveal`, `replace`, `patch`, and `close`.

`ui.tab(id)` is for durable workspace documents or trace/detail views. It
supports `open`, `reveal`, `replace`, `patch`, and `close`.

`ui.modal(id)` is for short confirmation and compact forms. It supports
`open`, `confirm`, and `close`. It is not a long-lived renderer host.

`ui.chat(threadId)` is a transcript surface. It appends `parts[]`: text,
markdown, activity, tool, data, source, error, step, and command/button parts.

`ui.statusItem(id)` is small status or command affordance in trusted shell
chrome.

`ui.renderer(id)` declares or references sandboxed rich rendering. Renderers are
escape hatches, not the common path.

## Payload Levels

Each surface can accept three payload levels:

1. Built-in Capsem blocks, rendered by trusted Svelte.
2. Declarative catalog updates, A2UI-like, rendered by trusted Svelte/native
   clients.
3. Sandboxed renderer references, ArrowJS/MCP-app-like, used only when the
   catalog cannot express the UI.

Example declarative catalog update:

```ts
await context.ui.sidePanel("capsem.gitContext").replace({
  catalog: "capsem.basic",
  surfaceId: "capsem.gitContext",
  data: {
    project: context.workspace.name,
    branch,
    worktree: context.workspace.root,
  },
  components: [
    { id: "root", component: "Card", children: ["title", "facts"] },
    { id: "title", component: "Text", text: "Git Context", variant: "title" },
    {
      id: "facts",
      component: "KeyValue",
      bind: { rows: "/gitFacts" },
    },
  ],
});
```

Internally this may lower to an A2UI-like sequence:

```ts
[
  { kind: "createSurface", surfaceId: "capsem.gitContext", catalogId: "capsem.basic" },
  { kind: "updateDataModel", surfaceId: "capsem.gitContext", data: { gitFacts } },
  { kind: "updateComponents", surfaceId: "capsem.gitContext", components },
]
```

This is the right bridge between model-authored UI and plugin-authored UI:
catalog components are expressive enough for rich UI but still validateable.

## One UI Family

Models, tools, security plugins, UI plugins, and built-ins should produce the
same UI family. The difference is authority:

- built-ins can call trusted shell internals
- plugins call the constrained `context.ui` object
- models produce validated UI objects through tools or render-capable outputs
- renderers receive private hydration data only after host authorization

That keeps Capsem from forking into one UI protocol for agents and another for
plugins.

## MVP Decision

The MVP should implement:

1. Manifest `contributes.ui.sidePanels`, `tabs`, `modals`, `statusItems`,
   `renderers`, `commands`, and `menus`.
2. Runtime surface handles: `ui.sidePanel(id)`, `ui.tab(id)`, `ui.modal(id)`,
   `ui.chat(id)`, `ui.statusItem(id)`.
3. Built-in structured block rendering in trusted Svelte.
4. A small Capsem catalog with A2UI-like component IDs, component names,
   children references, and data bindings.
5. Internal lowered `UiSurfaceOperation` for validation/logging/routing.
6. A later renderer-frame spike for ArrowJS-style custom rendering.

The first implementation should prove the API with a git context side panel:
detect `.git`, read `HEAD`, optionally fetch GitHub stats through the HTTPS
capability, and replace a side-panel block.
