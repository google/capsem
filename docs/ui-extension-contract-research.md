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

Obsidian itself is closed source, so the primary executable contract is the
published TypeScript API, sample plugin, and developer docs.

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
context.chat.stream.markdown("Policy found **3 issues**");
context.chat.stream.button({ command: "capsem.openFindings", title: "Open findings" });
context.chat.stream.filetree(files, "capsem://workspace");
context.chat.stream.render("application/vnd.capsem.findings+json", findings);
```

For rich printing, a plugin can contribute a renderer for a MIME-like payload:

```ts
UI("capsem.findings.renderer").chat_output_renderer({
  mimeTypes: ["application/vnd.capsem.findings+json"],
  render(data, webview, context) {
    const app = arrow(document.body);
    app.render(FindingTable(data));
    webview.postMessage({ type: "ready" });
  },
});
```

The renderer runs in a sandboxed webview/iframe with a `capsemApi` object,
similar to VS Code's `acquireVsCodeApi()`. Arrow-like libraries are fine inside
that UI sandbox; they should not grant more host authority.

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

Security plugins should not touch UI directly. They emit data:

```ts
context.ui.emit({
  channel: "workspace.context",
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
| UI renderer | Sandboxed webview/iframe JS with `postMessage` and state API | VS Code |
| Side panel | Declared view + provider/resolver + host-owned placement | VS Code + Obsidian |
| Chat rich output | Structured stream parts plus MIME renderer escape hatch | VS Code chat |
| Workspace panes | Leaf/sidebar ergonomics, reveal existing view before creating | Obsidian |
| Network | `fetch` capability and host permission policy | Chrome |
| Cleanup | Disposable/registered resources, cheap load, deferred heavy work | VS Code + Obsidian |

## Hard Design Questions Next

- Do UI renderers run as plain browser JS bundles while enforcement callbacks
  run as WASM, or do we require UI renderer logic to be compiled through the
  same TypeScript-to-WASM path and only generate declarative UI?
- What is the minimum safe "rich print" data model before allowing full
  webview renderers?
- Which surfaces are available in remote UI where the gateway may not have a
  native desktop shell?
- How do command buttons in chat route back through security enforcement before
  execution?
- How do we version UI block schemas so old plugins keep rendering?
