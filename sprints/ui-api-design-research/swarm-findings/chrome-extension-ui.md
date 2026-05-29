# Chrome Extension UI API Findings

Status: completed
Agent: Halley / `019e7492-ffcd-7552-90a8-8c45003fa6c2`
Scope: Inspect Chrome extension UI APIs and authoring vocabulary: manifest,
action, sidePanel, tabs, windows, commands, contextMenus, offscreen/documents
where relevant, message passing only as API context.

## Finding

Chrome's strongest lesson for Capsem is the split between static manifest
declarations and runtime surface methods. Chrome declares `action`,
`side_panel`, `commands`, `options_page` / `options_ui`, `permissions`, and
`host_permissions` in `manifest.json`, then uses focused runtime APIs like
`chrome.sidePanel.setOptions/open`, `chrome.tabs.create/query/sendMessage`,
`chrome.contextMenus.create`, and `chrome.runtime.sendMessage/connect`.

This maps cleanly to Capsem's existing direction: manifest `contributes`,
capability gates, and host-owned UI routing.

## Author-Facing Vocabulary

Use nouns and surfaces:

- `sidePanel`
- `command`
- `menu`
- `action`
- `options`
- `renderer`
- `tab`
- `modal`

Do not expose a generic `emit` as the authoring surface. If the host internally
lowers calls into messages or patches, that is runtime plumbing.

## Object Shapes To Copy

Chrome's option-object style is worth copying:

```ts
ui.sidePanel({
  id: "acme.gitContext",
  title: "Git Context",
  entry: "ui/git-context.html",
  location: "right",
  activation: { command: "acme.gitContext.open" },
  retainContextWhenHidden: false,
});

ui.modal({
  id: "acme.confirmRewrite",
  title: "Confirm Rewrite",
  entry: "ui/confirm-rewrite.html",
  size: { width: 560, height: 420 },
});

ui.tab({
  id: "acme.findings",
  title: "Findings",
  entry: "ui/findings.html",
  singleton: true,
});
```

Manifest declarations should include:

- `contributes.ui.sidePanels`
- `contributes.ui.modals`
- `contributes.ui.tabs`
- `contributes.commands`
- `contributes.menus`
- `permissions`
- `host_permissions`-style scoped capabilities, adapted to Capsem resources

Runtime should provide surface-specific verbs:

- `ui.sidePanel.open/update/close`
- `ui.modal.open/close`
- `ui.tab.open/reveal/update`

## What To Reject

- Do not copy `chrome.tabs` or `chrome.windows` directly as browser-control
  concepts. Capsem tabs/windows must mean real Capsem workspace surfaces.
- Do not model Capsem modals after Chrome action popups. Chrome popups
  auto-close and are toolbar-specific; Capsem needs durable modal semantics.
- Do not expose offscreen documents as an author API. Treat DOM-capable hidden
  renderers as host internals if needed.

## Capsem Implication

`ui.sidePanel` is the correct Chrome-aligned noun. `ui.modal` should be
Capsem-native, not `popup`. `ui.tab` is acceptable only if it means a Capsem
workspace or document tab, not a browser tab.

Transfer status: captured in sprint docs.
