# UI Extension Design Memo

## The Question

We need one way for models, plugins, tools, and built-ins to affect the UI.

The hard part is not whether the renderer is ArrowJS, Svelte, or something
else. The hard part is choosing the boundary:

- What can untrusted extension logic ask the UI to do?
- What can model output ask the UI to render?
- What stays in the trusted Capsem Svelte shell?
- When do we allow sandboxed custom rendering?

## What VS Code Teaches Us

VS Code's strongest idea is **declaration plus isolation**.

An extension declares contribution points in `package.json`: commands, menus,
views, chat participants, tools, webviews. At runtime, extension code registers
providers for those declared surfaces. Rich UI lives in webviews, which are
iframe-like isolated documents. The extension host talks to those webviews via
message passing. Webview assets go through `asWebviewUri`, HTML is expected to
carry a CSP, and retained hidden contexts are treated as expensive.

For Capsem, copy:

- manifest `contributes`
- declared views, commands, menus, chat renderers
- webview/iframe isolation for custom rich UI
- typed `postMessage`
- state restore instead of keeping hidden renderer contexts alive
- renderer registration keyed by `viewType` / MIME type

Do not blindly copy:

- VS Code's very broad extension API surface
- generic arbitrary command execution from UI buttons without Capsem policy
- old-style extension process trust assumptions

## What Obsidian Teaches Us

Obsidian's strongest idea is **workspace ergonomics**.

A plugin has `onload()` / `onunload()`. It registers commands, status bar
items, ribbon actions, settings tabs, and custom views. Views are attached to
`WorkspaceLeaf`s and can be revealed in left/right side panes. The mental model
is very approachable: register a view, find or create a side leaf, reveal it.

For Capsem, copy:

- `onload` as a lightweight registration phase
- `registerView`
- `ensureSideLeaf` / reveal existing panel before creating another
- view lifecycle: open, update, close
- side-panel ergonomics
- automatic cleanup of registered resources

Do not copy:

- ambient DOM/app authority
- direct plugin mutation of the trusted app shell
- expensive work or network fetches in load hooks

## What Capsem Already Has

Capsem's current UI is not a blank page:

- Trusted shell: Svelte 5 runes.
- Styling: Preline CSS tokens and component patterns, no Preline JS.
- App layout: browser-like shell with tabs, toolbar, VM views, settings.
- Existing side-panel pattern: settings sidebar, stats detail drawer, inspector.
- Existing isolation pattern: terminal iframe.
- Existing protocol pattern: typed parent/iframe `postMessage` with validation.
- Existing data plane: gateway HTTP/WebSocket plus bearer token.
- Existing visual language: dense operational UI, semantic tokens, blue/purple
  decision colors, no ad hoc plugin styling in the app shell.

That means Capsem should not let plugins mount arbitrary Svelte components
inside the trusted app. We should generalize the terminal iframe/message model
into an extension renderer model.

## Proposed Design

Use one UI object family:

```ts
type UiContribution =
  | ChatPart
  | SidePanelPatch
  | StatusItemPatch
  | CommandContribution
  | MenuContribution
  | RendererContribution;
```

Every producer emits this:

- model output
- tool result
- security plugin
- UI plugin
- built-in Capsem feature

The host validates, authorizes, logs, and routes it. The trusted Svelte shell
then either renders it with built-in components or mounts a sandboxed renderer.

```text
producer
  -> UiContribution
  -> host validation + policy + trace
  -> route: chat | side panel | status | trace | sandbox renderer
  -> trusted Svelte shell owns placement and chrome
```

## Concrete Surfaces

### Chat Windows

Chat should accept structured parts first:

```ts
context.ui.emit({ surface: "chat", kind: "markdown", body: "Found **3** issues" });
context.ui.emit({ surface: "chat", kind: "button", title: "Open findings", command: "capsem.findings.open" });
context.ui.emit({ surface: "chat", kind: "filetree", baseUri: "capsem://workspace", files });
context.ui.render({ surface: "chat", mime: "application/vnd.capsem.findings+json", data: findings });
```

This matches VS Code chat: markdown, buttons, file trees, references, progress,
and custom renderers. The model does not get a separate "draw UI" lane. It emits
the same contribution objects as plugins.

### Side Panels

Side panels should feel closer to Obsidian:

```ts
UI("capsem.git-context").view({
  id: "capsem.gitContext",
  title: "Git Context",
  location: "right",
  activation: ["onFileCreate"],
});
```

At runtime:

```ts
context.ui.emit({
  surface: "side_panel",
  target: "capsem.gitContext",
  operation: "upsert_block",
  block: {
    kind: "git-context-card",
    id: "git-context:workspace",
    title: "workspace",
    subtitle: "main",
    body: "stars 123 / forks 45 / issues 6"
  }
});
```

The shell decides whether that becomes a right drawer, a tab panel, or a remote
UI block. The extension does not decide layout by DOM mutation.

### Custom Renderers

A renderer is an implementation detail behind `context.ui.render(...)`.

```json
{
  "contributes": {
    "renderers": [
      {
        "id": "capsem.findings.renderer",
        "mimeTypes": ["application/vnd.capsem.findings+json"],
        "runtime": "arrow-js"
      }
    ]
  }
}
```

The host mounts a sandboxed iframe and sends:

```ts
{
  type: "render",
  contributionId,
  mime,
  data,
  theme,
  tokens,
  capabilities
}
```

Renderer replies through typed messages:

```ts
{ type: "ready" }
{ type: "height", value: 320 }
{ type: "command", command: "capsem.findings.open", args: {...} }
{ type: "error", message: "..." }
```

Commands emitted by renderers go back through host policy. They do not directly
call trusted app APIs.

## Where ArrowJS Fits

ArrowJS is interesting because it is tiny, plain JavaScript, template-literal
based, and model-friendly. That makes it a good candidate for the first
sandbox renderer runtime.

It should not replace Capsem's Svelte shell.

It should not create a second UI protocol.

It can fit as:

```text
UiContribution -> sandbox iframe -> ArrowJS renderer runtime -> DOM inside iframe
```

The same slot could later use:

- a smaller Capsem-authored renderer,
- a Svelte-compiled renderer,
- a declarative built-in Svelte renderer,
- or no custom runtime at all.

ArrowJS risk to track: it supports direct DOM/template rendering and IDL
property bindings such as `.innerHTML`. That is acceptable only inside the
sandbox renderer boundary, with CSP, no ambient network, no broad storage, and
typed messages.

## Tradeoff Table

| Axis | VS Code | Obsidian | Current Capsem | Proposed Capsem |
| --- | --- | --- | --- | --- |
| Extension declaration | Strong manifest contribution points | Simple manifest, many runtime registrations | No extension UI manifest yet | Chrome/VS Code-style `contributes` |
| Rich UI | Webview isolation | Direct DOM in app view | Trusted Svelte components, terminal iframe | Built-ins in Svelte, custom renderers in sandbox |
| Side panels | Declared views/providers | Excellent leaf/sidebar ergonomics | Settings/stats/inspector patterns | Declared views with Obsidian-like reveal behavior |
| Chat output | Structured stream + custom renderers | Not central | Not plugin-ready | Structured `ChatPart` + MIME renderers |
| Security boundary | Better: iframe/message/CSP | Weak: ambient app/DOM | Good for terminal iframe | Generalize iframe/message boundary |
| Model friendliness | Medium | Medium | Low for dynamic UI | High via typed contributions, optional renderer |
| Style consistency | Good but VS Code-specific | Theme-dependent DOM | Strong Preline tokens | Shell owns chrome; renderers get token bridge |

## Design Choice

MVP should be:

1. Define `UiContribution` schemas.
2. Render common contributions in trusted Svelte:
   chat markdown, buttons, file tree, cards, side-panel blocks, status items.
3. Generalize the terminal iframe protocol into `RendererFrame`.
4. Add one experimental renderer runtime, likely ArrowJS first because it is
   small and model-authorable.
5. Keep the renderer runtime replaceable.

This gives us one path. Models and plugins both speak `UiContribution`. The
renderer is a target, not a parallel architecture.

## Open Decisions

- What is the first exact `UiContribution` schema?
- Which contribution kinds are safe enough for model-generated output by
  default?
- Do renderer bundles come only from installed extensions, or can a model
  propose a temporary renderer in dev mode?
- How does remote UI render the same side-panel contributions?
- How much of Preline's token system do we expose into renderer iframes?
