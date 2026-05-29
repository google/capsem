# Capsem UI Fit Findings

Status: completed
Agent: Cicero / `019e7493-4d61-77b2-97c8-0d0e8c60246e`
Scope: Inspect current Capsem frontend from the main Capsem worktree:
Svelte 5, Preline, App shell, VMFrame/TerminalFrame typed postMessage, side
panels/drawers, Stats/Files/Inspector patterns, and implications for a single
UI authoring API.

## Sources Inspected

- Shell: `App.svelte`, `Toolbar.svelte`, `TabBar.svelte`, `tabs.svelte.ts`
- Frame/protocol boundary: `VMFrame.svelte`, `TerminalFrame.svelte`,
  `postmessage.ts`, `terminal.astro`
- Gateway/API stores: `api.ts`, `gateway.svelte.ts`, `vms.svelte.ts`,
  `gateway.ts`
- Existing surfaces: `SettingsPage.svelte`, `StatsView.svelte`,
  `FilesView.svelte`, `InspectorView.svelte`, `Modal.svelte`
- Theme/style: `global.css`, `theme.svelte.ts`

Static pass only. No files edited by the agent and no tests run.

## Finding

Capsem already has host-owned UI surfaces: VM tabs and singleton tabs in the
tab store, VM view switching in the toolbar, left nav panels in Settings and
Stats, right detail drawers in Stats, split panes in Files, host-owned modals,
and a typed terminal iframe protocol.

The API should let plugins address those nouns. It should not make authors send
raw transport messages.

## Recommended Surface Vocabulary

```ts
ui.sidePanel(...);
ui.modal(...);
ui.tab(...);
ui.chat(...);
ui.statusItem(...);
ui.renderer(...);
```

## Recommended Declaration Shape

```ts
contributes: {
  ui: {
    sidePanels: [
      { id, title, scope: "vm" | "workspace" | "global", placement: "left" | "right", renderer?: string }
    ],
    tabs: [
      { id, title, scope: "vm" | "global", renderer?: string }
    ],
    statusItems: [
      { id, label, scope: "vm" | "global", command?: string }
    ],
    renderers: [
      { id, mimeTypes, runtime: "arrow-js" | "declarative" }
    ]
  }
}
```

## Recommended Runtime Shape

```ts
await ui.sidePanel("capsem.findings").replace({
  scope: { vmId },
  title: "Findings",
  blocks: [
    { kind: "summary", title: "Policy findings", tone: "warning", value: 3 },
    { kind: "table", columns, rows },
    { kind: "button", label: "Open trace", command: "capsem.trace.open", args: { traceId } },
  ],
});

await ui.tab("capsem.trace").open({
  scope: { vmId },
  title: "Trace",
  renderer: { mime: "application/vnd.capsem.trace+json", data },
});

await ui.modal.confirm({
  title: "Apply policy change?",
  body: [{ kind: "text", text: "This will reload running sessions." }],
  confirm: { label: "Apply", command: "capsem.policy.apply" },
});

await ui.chat(threadId).append({
  parts: [
    { kind: "markdown", text: "Found **3** issues." },
    { kind: "button", label: "Show findings", command: "capsem.findings.reveal" },
  ],
});
```

## What To Copy

- Copy the terminal iframe boundary pattern: initial state by URL/config,
  runtime updates by typed `postMessage`, validators on both sides, explicit
  message unions.
- Copy host-owned chrome. Extensions provide blocks, renderers, commands, and
  state; Svelte owns placement, tabs, toolbar, sidebars, and modal frames.
- Copy VM scoping. Many surfaces should accept `{ vmId }`.
- Copy Preline token discipline. Built-in blocks render with semantic classes
  and theme state, not plugin-supplied styling.

## What To Reject

- Direct plugin Svelte component mounting inside the trusted shell.
- Arbitrary DOM/HTML in App, Toolbar, tabs, Settings, Stats, or Files.
- Preline JS/data attributes and arbitrary Tailwind class strings from plugins.
- Renderer access to the module-scoped gateway bearer token.
- Long-lived rich UI in modals. Modals fit confirmation and compact forms.
- `ui.emit(...)` as author API. It can exist only as host plumbing.

## Capsem Implication

Use a two-tier model: trusted built-in Svelte block rendering for common UI,
plus sandboxed renderer iframes for rich/custom UI. `ui.sidePanel`, `ui.tab`,
`ui.chat`, and `ui.statusItem` should mostly accept structured blocks.
`ui.renderer` is the escape hatch, implemented with a generalized
`RendererFrame` based on the existing VM/terminal frame pattern.

Open questions:

- exact block schema
- whether renderer network is always denied by default
- how remote UI maps side panels
- whether `ui.tab(...)` creates VM-scoped view tabs or only global singleton
  tabs

Transfer status: captured in sprint docs.
