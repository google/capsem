# Agent UI Protocol Findings

Status: completed
Agent: Laplace / `019e7493-17e4-79d3-b87e-e92c47471e6e`
Scope: Inspect current agent-generated UI protocols and libraries, especially
AG-UI if relevant, OpenAI Apps SDK/tool-output-template style UI, Vercel AI SDK
generative UI if relevant, and any protocol where AI/tool output maps to UI.

## Sources Inspected

- AG-UI events, messages, tools, and generative UI draft.
- OpenAI Apps SDK / MCP Apps reference, ChatGPT UI guide, MCP server guide,
  MCP Apps overview/build docs.
- Vercel AI SDK UI generative UI, UIMessage, chatbot tool usage, streaming
  custom data, stream protocol, `streamUI` reference.

Static research pass. No files edited by the agent.

## AG-UI

Core shapes:

- event stream with `{ type, timestamp?, rawEvent? }`
- message lifecycle keyed by `messageId`
- tool call lifecycle keyed by `toolCallId`
- state/activity snapshots and deltas
- JSON Patch for live state updates
- frontend-defined tools with JSON Schema parameters

How UI is generated:

- Stable AG-UI maps agent/tool events to frontend-rendered messages, activity
  views, and state.
- Direct generative UI is still draft and uses a secondary UI generator tool.

Copy:

- snapshot/delta model
- frontend-only activity messages
- explicit tool-call lifecycle and IDs
- JSON Patch for live UI updates

Reject:

- treating the generative UI draft as stable
- letting the main agent generate arbitrary React/HTML directly
- making raw/custom events the normal extension path

## OpenAI Apps SDK / MCP Apps

Core shapes:

- tool descriptor links UI through resource metadata
- UI resource is an HTML template/resource, usually with app-specific metadata
- tool result splits payloads into:
  - `structuredContent`: model-visible and component-visible JSON
  - `content`: model-visible transcript content
  - `_meta`: component-only hydration data hidden from the model
- bridge concepts let components receive tool input/result and call tools back
  through a controlled host channel

How UI is generated:

- The model selects or calls a tool.
- A render-capable tool points to a predeclared UI template.
- The template renders the structured tool result.
- OpenAI recommends separating data tools from render tools.

Copy:

- model-visible data vs renderer-only private data
- renderer/resource declaration as metadata, not inline UI generation
- data tools vs render tools
- controlled bridge for component-to-tool calls

Reject:

- ChatGPT-specific metadata keys as Capsem core protocol
- binding author APIs to iframe, JSON-RPC, or MIME ceremony
- attaching UI templates to every data tool by default

## Vercel AI SDK UI

Core shapes:

- `UIMessage { id, role, metadata?, parts[] }`
- parts for text, reasoning, sources, files, steps, tool calls, and custom data
- tool parts keyed by `toolCallId` and explicit states
- `data-*` custom UI/data parts with ID-based reconciliation
- stream protocol with start/delta/end text, tool input/output, data, errors,
  finish/abort

How UI is generated:

- Tool results map to app-owned components.
- The model chooses tools; app code renders `message.parts`.
- `streamUI` can return React nodes but is not the right Capsem foundation.

Copy:

- `UIMessage.parts[]` as durable render transcript
- tool state machine
- custom rich data parts
- ID-based reconciliation
- validation of persisted UI messages against schemas

Reject:

- React nodes as protocol objects
- RSC `streamUI` as a foundation
- encoding every long-term Capsem domain concept as a tool-name string type

## Capsem API Implication

Keep the public API noun/surface-first and compile it into a protocol object.

```ts
ui.sidePanel("trace-inspector", {
  title: "Trace Inspector",
  renderer: "capsem://renderers/trace-inspector",
  props: { runId },
  modelContent: { summary, selectedIds },
  privateContent: { rowsById, rawEvents },
});
```

Recommended lowered shape:

```ts
{
  kind: "ui.surface",
  action: "open" | "update" | "close",
  surface: "sidePanel" | "modal" | "tab" | "chat",
  id: string,
  renderer?: { uri: string, version?: string },
  title?: string,
  props?: object,
  modelContent?: object,
  privateContent?: object,
  update?: { mode: "snapshot" | "patch", patch?: JsonPatch[] },
}
```

For `ui.chat`, use a `UIMessage.parts[]`-style transcript with parts such as
`text`, `tool`, `data`, `source`, `activity`, `error`, and `step`.

The design move is to keep authoring surfaces stable and transport-free. Copy
OpenAI's renderer/data privacy split, Vercel's message parts and state machine,
and AG-UI's snapshot/delta/activity vocabulary.

Transfer status: captured in sprint docs.
