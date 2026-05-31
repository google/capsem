# UI API Authoring Synthesis

## Correction

`emit` is not an authoring API. It is an internal lowering/transport operation.

`renderer` is also the wrong public primitive if it means arbitrary UI code.
Capsem should not have an open-ended renderer escape hatch. The public API must
be made of named UI surfaces and catalog-driven payloads:

```ts
ui.sidePanel(...);
ui.modal(...);
ui.tab(...);
ui.chat(...);
ui.statusItem(...);
ui.catalog(...);
```

Authors should think like Chrome extension authors: declare surfaces,
commands, menus, tools, and catalogs in a manifest, then call focused methods
on those surfaces. They should not think in host messages, sockets, iframes,
DOM, HTML, CSS, or patch routing.

## Design Rule

There is one UI engine for Capsem:

```text
SurfaceModel + Catalog + DataModel + Action
```

Every UI surface uses that engine:

- chat windows
- side panels
- tabs
- modals
- status items
- remote UI cards
- plugin-authored UI contributions
- model-authored UI contributions

Third-party code never receives DOM authority. If a plugin contributes a
custom component, it contributes a schema and optionally a sandboxed resolver
that returns lower-level catalog nodes. The resolver follows the same wall as
security plugins:

```text
resolveComponent(component_copy, context_copy) -> component_tree_copy
```

Only the returned catalog tree crosses back. The host validates it before the
trusted Svelte renderer paints anything.

## Research Synthesis

Chrome teaches the strongest extension shape: static manifest declarations plus
runtime noun APIs. Copy `sidePanel`, `commands`, `contextMenus`, `action`,
permissions, and option-object methods. Do not copy browser-specific
`tabs/windows` semantics unless Capsem has a real corresponding workspace
surface.

VS Code gives contribution taxonomy: commands, menus, views, chat
participants, tools, and declared surface IDs. It also shows that chat should
stream structured parts: markdown, progress, buttons, tool states, followups,
and references. Copy the taxonomy, not webviews as an authoring escape hatch.

Obsidian gives workspace ergonomics: register a view, find or create a side
leaf, reveal it, and keep lifecycle cleanup obvious. Copy
`ensureSideLeaf`/reveal behavior through `ui.sidePanel(id).reveal()` and
`ui.tab(id).reveal()`, not ambient DOM authority.

A2UI is the strongest default shape for generated UI across trust boundaries:
declarative surfaces, component catalogs, data binding, flat adjacency-list
component graphs, streaming updates, and no arbitrary code execution. This
should be the center of Capsem's UI object model.

Rust crate check: crates.io currently has `a2ui-types`, `a2ui-core`, and
`a2ui-validation`. `a2ui-types` gives v0.8/v0.9 protocol message structs, but
v0.9 catalog components are still represented as JSON values, so Capsem still
needs strict Rust component enums/builders for A2UI Basic if we want
component-level type safety. The first prototype now cross-checks our messages
against `a2ui-types::v09::server_to_client::ServerToClientMessage`.

assistant-ui's generative UI spec is the small-tree version of the same idea:
a `generative-ui` message part carries JSON nodes, and a renderer resolves
component names against an allowlist. That is useful for chat-local simple
trees, but A2UI's flat model is better for long-lived surfaces and patches.

CopilotKit gives the product taxonomy: controlled components, declarative
catalog UI, and open-ended sandboxed UI. Capsem should deliberately reject the
open-ended lane for now. We keep controlled and declarative under one engine.

ArrowJS teaches authoring ergonomics: tiny concepts, plain TypeScript, and
model-friendly examples. Do not copy inline DOM authority. If Capsem uses
Arrow-like syntax, it should compile into sandboxed component resolvers that
return catalog nodes.

prompt-kit and motion-primitives are implementation references for first-party
chat blocks and interaction polish. They are not protocol surfaces.

## Catalog Types

Capsem should use catalogs as the extension point, not arbitrary renderers. The
first catalog is not invented by us: it is A2UI v0.9 Basic, ported as Rust
structs/enums and rendered by trusted Svelte. Capsem-specific catalogs come
after that baseline is proven.

| Catalog | Purpose | Third-Party? | Runtime |
| --- | --- | --- | --- |
| A2UI v0.9 Basic | Baseline protocol: `Text`, `Row`, `Column`, `Card`, `Button`, `Modal`, `Image`, `Icon`, `Divider`, data binding, functions, actions | No | Trusted Svelte/Preline |
| Chat baseline | Ordered transcript surfaces composed from A2UI Basic components | No | Same A2UI engine plus chat ordering |
| `capsem.security` | Security objects: `DecisionBadge`, `PolicyFinding`, `TraceLink`, `RiskMeter`, `EvidenceList` | No initially | Trusted Svelte or resolver to A2UI Basic |
| `capsem.workspace` | Workspace objects: `FileTree`, `GitContext`, `BranchBadge`, `WorktreeSummary` | Maybe by review | Trusted Svelte or resolver to A2UI Basic |
| `vendor.*` | Extension-specific semantic components | Yes, with capability review | Sandbox resolver to approved catalog nodes |

Custom catalog functions are pure and schema-validated. They may format data,
derive labels, or choose variants. They must not fetch, read files, mutate UI,
or call tools. If a function needs authority, it is a tool or plugin callback,
not a catalog function.

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
          catalog: "a2ui.basic.v0_9",
        },
      ],
      tabs: [],
      modals: [],
      statusItems: [],
      catalogs: [
        {
          id: "vendor.git",
          extends: "capsem.workspace",
          components: ["GitContextSummary"],
        },
      ],
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
      catalog: "a2ui.basic.v0_9",
      data: {
        project: context.workspace.name,
        branch,
        worktree: context.workspace.root,
      },
      components: [
        { id: "root", component: "Card", child: "body" },
        {
          id: "body",
          component: "Column",
          children: ["title", "branch", "worktree"],
        },
        { id: "title", component: "Text", text: { path: "/project" }, variant: "h3" },
        { id: "branch", component: "Text", text: { path: "/branch" }, variant: "body" },
        { id: "worktree", component: "Text", text: { path: "/worktree" }, variant: "caption" },
      ],
    });

    return file;
  });
```

The plugin callback still returns the object it received. UI work is a side
effect through a constrained context capability, but the authoring object is
`ui.sidePanel(...)`, not `emit`.

## Chat Shape

Chat is not a separate UI system. A chat message is an ordered surface update
using the same catalogs.

```ts
await context.ui.chat(threadId).append({
  catalog: "a2ui.basic.v0_9",
  parts: [
    { id: "root", component: "Card", child: "body" },
    { id: "body", component: "Column", children: ["tool", "summary"] },
    { id: "tool", component: "Text", text: "git.inspect completed", variant: "caption" },
    { id: "summary", component: "Text", text: "Found 3 workspace facts.", variant: "body" },
  ],
});
```

This lets models, tools, and plugins share the same block vocabulary. The
surface decides placement: chat bubble, right panel, workspace tab, modal body,
or remote UI card.

## Spreadsheet To Slide Deck Track

PM requirement: Capsem will eventually need to create spreadsheet data, derive
charts from that data, create diagrams, embed charts/images/text/diagrams into
slides, and combine slides into a slide deck artifact.

That is not part of the immediate plugin/WASM MVP, but it changes the UI
catalog design now:

- spreadsheet objects need typed sheets, named ranges, and addressable cell
  references so chart inputs can point at data instead of copying blobs
- little Codex needs a SQLite-backed scratch data workbench for joins,
  grouping, filtering, pivot-style summaries, and repeatable chart inputs;
  arrays in model context are not enough for finance/science/report workflows
- chart objects need stable export handles for PNG/SVG first, with PDF or deck
  export considered later
- slide objects need typed blocks for chart, image, text, table, and generated
  UI fragments
- diagram objects should start with a Mermaid-backed variant because Mermaid is
  text-native, reviewable, model-friendly, and exportable; the public API should
  still be `ui.diagram`, not `ui.mermaid`, so Graphviz or first-party diagram
  specs can be added later
- slide decks need ordered slide composition, metadata, and deterministic export
  separate from the live Svelte renderer
- `web.preview` needs its own later sprint as the browser-backed surface for
  showing user-visible pages, previews, and acceptance states; it is related to
  browser automation, but the product object is a preview surface, not raw
  browser control
- generated media should be first-class typed Capsem assets via
  `generate.image`, `generate.video`, and `generate.audio`, so models can create
  media for cards, slides, decks, and chat without stuffing blobs into UI specs
- the same Rust catalog types should feed chat, side panels, slides, and deck
  export so model-authored UI does not fork from plugin-authored UI

The concrete spike is tracked in
`sprints/capsem-native-mcp-and-deck-proof/`: prove the end-to-end path from
SQLite data to generated media, chart/diagram specs, slide composition, deck
export, and `web.preview`. Before MCP wiring, a small Python client should call
the Rust webserver routes for each native API so the final MCP tools wrap a
known surface instead of inventing one.

SQLite is not the public UI API, and it should not be scoped only to slides. It
is a general per-instance SQL workspace behind the tools/plugins/model lane.
Each agent/session/workspace instance can receive its own constrained SQLite
database for shaping data before it becomes UI, telemetry, reports, or export
artifacts.

Because Capsem already exposes its own local MCP, the first implementation
should present this as MCP tools for little Codex rather than as a bespoke UI
feature:

```text
local__data_sqlite_create
local__data_sqlite_replace_table
local__data_sqlite_query
local__data_sqlite_to_sheet
local__ui_render
```

The same Rust service can later expose equivalent plugin context methods, but
the MCP lane lets agents manipulate data and prove the chart/slide contract
without waiting for the full WASM plugin runtime.

This per-instance SQL lane should be useful beyond deck generation: finance
analysis, scientific tables, benchmark summaries, audit traces, session
inspection, and any workflow where a model needs reliable joins, filters,
aggregations, or reusable intermediate tables.

```ts
const db = context.data.sqlite("board_packet");

await db.table("quarterly_metrics").replace({
  columns: [
    { name: "quarter", type: "text" },
    { name: "revenue", type: "number" },
    { name: "margin", type: "number" },
  ],
  rows,
});

const summary = await db.query(`
  select quarter, revenue, margin
  from quarterly_metrics
  order by quarter
`);
```

The query result can then lower into `ui.sheet`, `ui.table`, charts, and slide
blocks. This gives little Codex a disciplined place to manipulate data while
keeping the render/export contract typed and auditable.

Generated media should lower into the asset store before it enters UI:

```ts
const hero = await generate.image("capsem_architecture_hero", {
  prompt: "clean technical illustration of an isolated security workspace",
  aspectRatio: "16:9",
  inputs: [
    { kind: "image", asset: "capsem://asset/reference_dashboard" },
  ],
});

const narration = await generate.audio("deck_intro_voiceover", {
  text: "This deck summarizes the Q4 security posture.",
  voice: "neutral",
  inputs: [
    { kind: "audio", asset: "capsem://asset/reference_voice" },
  ],
});
```

Those handles can then be used by `ui.imageBlock`, future `ui.videoBlock`, and
future `ui.audioBlock`, or exported into slides and decks. The generation APIs
produce typed assets; they do not bypass the renderer, asset store, or export
validation.

First spike: implement the `generate` lane through Capsem MCP tools backed by
Gemini, using the existing Capsem Gemini API key when it is present. If the key
is unavailable, the tool should fail explicitly with a configuration error
instead of silently selecting a different provider.

Longer term, provider configuration is its own design question:

- how users choose OpenAI, Gemini, local, or other media providers
- which providers support image, video, audio, and multimodal inputs
- how the UI displays provider availability, cost/latency, and failure state
- how generated assets record provider/model/provenance without leaking secrets

Generation tools need multimodal inputs from day one. Image generation may take
reference images, masks, sketches, or screenshots. Video generation may take
images, clips, timing hints, and storyboards. Audio generation may take text,
voice references, existing audio, or timing cues. All inputs should be asset
handles or typed inline content accepted by the Capsem MCP tools, not ad hoc
paths or raw blobs.

As the local MCP grows, tools should be grouped by product nouns:

```text
local__data_sqlite_*
local__ui_*
local__generate_image
local__generate_video
local__generate_audio
local__asset_*
local__export_*
local__web_preview_*
```

This follows the real Capsem guest MCP convention: tools are provided by the
local built-in MCP server and exposed through the aggregator as
`server__tool_name`; the current built-in server is named `local`
(`local__fetch_http`, `local__snapshots_list`, etc.). Host-control tools in
`capsem-mcp` use `capsem_*` names for VM/session operations and should not
become the shape of the in-guest product tools.

The pretty product API remains `generate.image`, `ui.barChart`, and
`web.preview`. MCP wire names are snake_case and namespaced; authoring APIs are
typed nouns.

`web.preview` should be planned as a separate surface:

```ts
await web.preview("board_packet_preview").show({
  artifact: "capsem://deck/board_packet",
  mode: "interactive",
});
```

This is the lane for user-visible browser previews and acceptance checks. It
should reuse Capsem's browser capability underneath, but the authoring object
is preview/show/inspect, not a grab bag of browser verbs.

Candidate authoring shape:

```ts
const sheet = ui.sheet("quarterly_metrics", summary);

const chart = ui.barChart("revenue_by_quarter", {
  source: sheet.range("A1:C5"),
  x: "quarter",
  series: ["revenue", "margin"],
  stack: false,
});

const flow = ui.diagram("review_flow", {
  kind: "mermaid",
  source: `
    flowchart LR
      A[Model call] --> B{Policy review}
      B -->|allow| C[Continue]
      B -->|review| D[Ask user]
  `,
});

const intro = ui.slide("exec_intro", {
  blocks: [
    ui.textBlock({ title: "Q4 Security Posture", body: "Revenue and risk view." }),
    ui.chartBlock(chart),
    ui.diagramBlock(flow),
    ui.imageBlock({ src: "capsem://asset/logo" }),
  ],
});

await ui.slideDeck("board_packet").replace({ slides: [intro] });
```

This track belongs beside chart/diagram/export work, not after it. If charts
and diagrams cannot round-trip from typed Rust spec to Svelte preview to
deterministic exported asset, they will not be usable in decks.

## Lowered Host Object

Surface calls compile into internal operations like this:

```ts
[
  {
    kind: "createSurface",
    surface: "sidePanel",
    surfaceId: "capsem.gitContext",
    catalogId: "capsem.workspace",
  },
  {
    kind: "updateDataModel",
    surfaceId: "capsem.gitContext",
    data: { project, branch, worktree },
  },
  {
    kind: "updateComponents",
    surfaceId: "capsem.gitContext",
    components: [
      { id: "root", component: "GitContext", bind: { branch: "/branch" } },
    ],
  },
]
```

This object is for validation, logging, remote UI routing, and rendering. It is
not the API we teach plugin authors.

## Surface Semantics

`ui.sidePanel(id)` is for durable contextual workspace or VM panels. It supports
`open`, `reveal`, `replace`, `patch`, and `close`.

`ui.tab(id)` is for durable workspace documents or trace/detail views. It
supports `open`, `reveal`, `replace`, `patch`, and `close`.

`ui.modal(id)` is for short confirmation and compact forms. It supports
`open`, `confirm`, and `close`. It is not a long-lived app host.

`ui.chat(threadId)` is a transcript surface. It appends ordered catalog parts:
text, markdown, activity, tool, data, source, error, step, and command/button
parts.

`ui.statusItem(id)` is small status or command affordance in trusted shell
chrome.

`ui.catalog(id)` references a component/function catalog declared by the
manifest.

## One UI Family

Models, tools, security plugins, UI plugins, and built-ins should produce the
same UI family. The difference is authority:

- built-ins can call trusted shell internals
- plugins call the constrained `context.ui` object
- models produce validated catalog operations through tools or model output
- custom components can only run sandboxed resolvers that return catalog nodes

That keeps Capsem from forking into one UI protocol for agents and another for
plugins.

## MVP Decision

The MVP should implement:

1. Manifest `contributes.ui.sidePanels`, `tabs`, `modals`, `statusItems`,
   `catalogs`, `commands`, and `menus`.
2. Runtime surface handles: `ui.sidePanel(id)`, `ui.tab(id)`, `ui.modal(id)`,
   `ui.chat(id)`, `ui.statusItem(id)`.
3. A2UI v0.9 Basic, rendered in trusted Svelte/Preline.
4. A2UI Basic component IDs, component names, children references, data binding,
   and actions, ported directly before Capsem-specific catalogs are added.
5. Internal lowered `UiSurfaceOperation` for validation/logging/routing.
6. A sandboxed component resolver ABI that can only return validated catalog
   nodes.

The first implementation should prove the API with a git context side panel
and a chat message using the same catalog engine: detect `.git`, read `HEAD`,
optionally fetch GitHub stats through the HTTPS capability, update the side
panel, and append a chat part.
