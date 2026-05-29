# UI Extension Contract Research

## Source Snapshot

Local ignored checkouts live under `private/upstream/`.

| System | Source | Revision |
| --- | --- | --- |
| VS Code | `microsoft/vscode` | `32179cb` |
| VS Code samples | `microsoft/vscode-extension-samples` | `61fda64` |
| Obsidian API | `obsidianmd/obsidian-api` | `2e88986` |
| Obsidian sample plugin | `obsidianmd/obsidian-sample-plugin` | `dc2fa22` |
| Obsidian developer docs | `obsidianmd/obsidian-developer-docs` | `96e5dae` |
| ArrowJS | `standardagents/arrow-js` | `f134a77` |
| Capsem frontend | local `frontend/` | Svelte 5 + Preline |

Obsidian itself is closed source, so the primary executable contract is the
published TypeScript API, sample plugin, and developer docs.

## Decision Frame

The core design constraint is **one UI contribution path**.

Models, rules, security plugins, tools, and UI plugins should not get separate
ways to change the application. They should all produce the same host-validated
UI contribution objects. The host then routes those objects into chat, side
panels, status items, trace views, or renderer sandboxes.

That means ArrowJS is only a directional reference for authoring ergonomics:
tiny JavaScript, no heavy compiler, direct DOM rendering, easy for a model to
write. It is not the architecture. We may choose ArrowJS, a lighter Svelte-like
runtime, or a Capsem-authored micro renderer later. The architecture is the UI
contribution protocol.

Capsem already has a trusted app shell:

- Svelte 5 runes own app state and trusted first-party interactivity.
- Preline is used for CSS tokens and component patterns only.
- Preline JS plugins are not allowed.
- The terminal already uses a sandboxed iframe plus typed `postMessage`.
- Existing views use dense Svelte panels, side panes, detail drawers, Shiki
  highlighting, LayerChart, and gateway/WebSocket data flows.

So the question is not "ArrowJS versus Svelte for the whole UI." The question
is what runtime, if any, should execute inside **extension-owned surfaces**
after the host has accepted a typed UI contribution.

## Design Recommendation

Build a single `UiContribution` pipeline:

```ts
type UiContribution =
  | ChatPart
  | UiPatch
  | SidePanelContribution
  | StatusItemContribution
  | CommandContribution
  | RendererContribution;
```

All producers use it:

```ts
context.ui.emit({
  surface: "chat",
  kind: "markdown",
  body: "Policy found **3 issues**"
});

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

context.ui.render({
  surface: "chat",
  mime: "application/vnd.capsem.findings+json",
  data: findings
});
```

The same object shape should be available to:

- model output rendering,
- security plugin callbacks,
- tool results,
- extension UI plugins,
- built-in Capsem features.

The trusted Svelte app consumes this protocol first. When a contribution needs
arbitrary rich UI, the host mounts a sandboxed renderer surface that also
receives the same contribution object. No producer gets to bypass the protocol.

## Runtime Tradeoff

| Option | What It Means | Strengths | Risks | Decision |
| --- | --- | --- | --- | --- |
| Svelte-only shell | Plugins/models can only emit objects rendered by first-party Svelte components | Maximum consistency with Capsem Svelte/Preline; smallest attack surface | Too slow for third-party rich UI; every new renderer requires Capsem code | Use for default built-in blocks |
| Declarative schema only | Contributions are JSON UI trees, no plugin JS renderer | Very auditable; easy to validate; remote UI friendly | Can become a bad private UI language; limited for charts/interactive output | Good MVP floor, not enough long term |
| ArrowJS sandbox renderer | Approved renderer runtime inside iframe/webview | Tiny runtime; plain JS; model-friendly; no build step; good analogy for rich printing | Another reactive model beside Svelte; must constrain `.innerHTML`, network, storage, and messages | Viable first renderer experiment, not core architecture |
| Lighter Svelte-like runtime | Capsem-authored or Svelte-compiled minimal renderer for extensions | Closer to our app mental model; one syntax family | Compiler/runtime work; may be heavier than needed; harder for models to generate | Worth a spike if ArrowJS ergonomics are not enough |
| Full Svelte plugins | Third-party/plugin UI compiled as Svelte components | Best fit with current app stack | Trust boundary is harder; bundle/version coupling; app-shell integration risk | Avoid for untrusted plugins initially |

Recommendation: start with **declarative blocks plus one sandbox renderer
runtime**. The renderer runtime is behind the `RendererContribution` contract,
so choosing ArrowJS now does not fork the product. It is replaceable if a
lighter Svelte-like runtime proves better.

## VS Code Findings

VS Code's model is a strong fit for Capsem's security posture:

- Static `package.json` contribution points declare views, commands, menus,
  chat participants, tools, and webview surfaces.
- Extension code registers providers at activation time.
- Rich UI runs in a webview iframe, not in the workbench DOM.
- The extension host and webview talk over JSON-serializable `postMessage`.
- Webview resources must be translated through `webview.asWebviewUri(...)`.
- Webview HTML is expected to use a strict content security policy and nonce.
- Hidden webviews are normally torn down; persistence should use serialized
  state rather than retaining heavy UI contexts.

Primary source anchors:

- `WebviewPanel`, `WebviewView`, persisted state, and provider contracts:
  `private/upstream/vscode/src/vscode-dts/vscode.d.ts`.
- `window.createWebviewPanel(...)`, `registerWebviewViewProvider(...)`,
  `registerTreeDataProvider(...)`, and `createStatusBarItem(...)`:
  `private/upstream/vscode/src/vscode-dts/vscode.d.ts`.
- Webview host implementation serializes messages, tracks disposal, rewrites
  resource URIs, and warns on missing CSP:
  `private/upstream/vscode/src/vs/workbench/api/common/extHostWebview.ts`.
- The webview sample shows command registration, one-panel management,
  serializer registration, `enableScripts`, `localResourceRoots`, nonce/CSP,
  and message passing.
- The webview-view sample shows a sidebar view declared in `contributes.views`
  and fulfilled by `registerWebviewViewProvider(...)`.

VS Code chat is especially relevant for our "rich printing" problem:

- `ChatResponseStream` supports structured response parts: markdown, anchors,
  command buttons, file trees, progress, references, and generic parts.
- Proposed `ChatOutputRenderer` registers renderers for MIME-like payloads and
  renders them into webviews. That is the cleanest analogy for "model prints a
  chart/rich object into chat."

## Obsidian Findings

Obsidian's model is a strong fit for Capsem's workspace and side-panel
ergonomics:

- A plugin is a class with `onload()` / `onunload()` lifecycle.
- UI primitives are methods on `Plugin`: `addRibbonIcon`,
  `addStatusBarItem`, `addCommand`, `addSettingTab`, and `registerView`.
- A custom side pane is an `ItemView` created by a `ViewCreator` and placed in
  a `WorkspaceLeaf`.
- `Workspace.ensureSideLeaf(...)`, `getLeavesOfType(...)`, `setViewState(...)`,
  and `revealLeaf(...)` are the core pane placement/reveal techniques.
- Obsidian emphasizes automatic cleanup via plugin registration helpers and
  light `onload()` work.

Primary source anchors:

- `Plugin`, lifecycle, commands, status bar, ribbon, settings, view
  registration: `private/upstream/obsidian-api/obsidian.d.ts`.
- `PluginManifest`: identity, version, minimum app version, description,
  desktop-only flag.
- `ItemView`, `WorkspaceLeaf`, `ensureSideLeaf(...)`, and `setViewState(...)`:
  `private/upstream/obsidian-api/obsidian.d.ts`.
- The sample plugin shows idiomatic `onload()` registrations for ribbon,
  status bar, commands, settings, DOM events, and intervals.
- Developer docs warn that `onload()` should only register capabilities and
  should avoid expensive computation or data fetching.

Obsidian is less suitable as a security model because plugins receive direct
DOM and app access. We should copy its ergonomic pane concepts, not its ambient
authority.

## Capsem Direction

Capsem should use one extension package model with multiple contributions:

```json
{
  "manifest_version": 1,
  "name": "capsem.git-context",
  "permissions": ["fs.read", "fetch", "ui.emit"],
  "host_permissions": ["https://api.github.com/*"],
  "contributes": {
    "commands": [],
    "views": [],
    "chatRenderers": [],
    "chatParticipants": [],
    "menus": [],
    "statusItems": [],
    "renderers": [],
    "plugins": [],
    "rules": [],
    "tools": [],
    "skills": []
  }
}
```

Keep the authoring vocabulary close to JavaScript, Chrome extensions, and VS
Code:

- `manifest.json`
- `permissions`
- `host_permissions`
- `contributes`
- `commands`
- `views`
- `menus`
- `webviews`
- `fetch`
- callback hooks
- `postMessage`
- `getState` / `setState`

## Proposed UI Surfaces

### Chat Output

Default chat output should be structured parts, not arbitrary HTML:

```ts
context.ui.emit({ surface: "chat", kind: "markdown", body: "Policy found **3 issues**" });
context.ui.emit({ surface: "chat", kind: "button", command: "capsem.openFindings", title: "Open findings" });
context.ui.emit({ surface: "chat", kind: "filetree", files, baseUri: "capsem://workspace" });
context.ui.render({ surface: "chat", mime: "application/vnd.capsem.findings+json", data: findings });
```

For rich printing, an extension can contribute a renderer for a MIME-like
payload. ArrowJS is an example runtime, not a special path:

```ts
UI("capsem.findings.renderer").renderer({
  mimeTypes: ["application/vnd.capsem.findings+json"],
  render(data, webview, context) {
    renderFindingTable(data, document.body);
    webview.postMessage({ type: "ready" });
  },
});
```

The renderer runs in a sandboxed webview/iframe with a `capsemApi` object,
similar to VS Code's `acquireVsCodeApi()`. The runtime may be ArrowJS, a
lighter Svelte-like runtime, or a Capsem runtime, but it receives the same
`UiContribution` and has the same `postMessage`/state constraints.

### Side Panels

Side panels should be manifest-declared and provider-resolved:

```ts
UI("capsem.git-context").view({
  id: "capsem.gitContext",
  name: "Git Context",
  location: "right",
  retainContextWhenHidden: false,
  resolve(panel, context) {
    panel.html = renderGitPanelShell();
    panel.onMessage((message) => context.commands.execute(message.command));
  },
});
```

The host owns placement, reveal, persistence, and disposal. The extension owns
content inside its surface.

### UI Mutation Channel

Security plugins, model output handlers, and tools should not touch UI
directly. They emit data:

```ts
context.ui.emit({
  surface: "side_panel",
  target: "workspace.context",
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

The gateway/UI decides how to route that mutation into chat, a side panel, a
status item, or a trace view.

## Recommended Split

| Concern | Recommended Capsem Model | Borrow From |
| --- | --- | --- |
| Extension package | Static manifest with `contributes`, `permissions`, `host_permissions` | Chrome, VS Code |
| Security logic | WASM callback receives copied object/context and returns object | Current prototype |
| UI protocol | One host-validated `UiContribution` object family for models and plugins | Capsem |
| UI renderer | Replaceable sandbox runtime behind `RendererContribution` | VS Code + ArrowJS direction |
| Side panel | Declared view + provider/resolver + host-owned placement | VS Code + Obsidian |
| Chat rich output | Structured stream parts plus MIME renderer escape hatch | VS Code chat |
| Workspace panes | Leaf/sidebar ergonomics, reveal existing view before creating | Obsidian |
| Network | `fetch` capability and host permission policy | Chrome |
| Cleanup | Disposable/registered resources, cheap load, deferred heavy work | VS Code + Obsidian |

## Hard Design Questions Next

- What is the smallest `UiContribution` schema that can serve both models and
  plugins without becoming a bad private UI language?
- Is ArrowJS good enough as the first sandbox renderer runtime, or should we
  build a lighter Svelte-like runtime that keeps the Capsem frontend mental
  model intact?
- What is the minimum safe "rich print" data model before allowing full
  webview renderers?
- Which surfaces are available in remote UI where the gateway may not have a
  native desktop shell?
- How do command buttons in chat route back through security enforcement before
  execution?
- How do we version UI block schemas so old plugins keep rendering?
