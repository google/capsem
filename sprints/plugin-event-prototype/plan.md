# Plugin Event Prototype Sprint

## Goal

Build an isolated prototype for Capsem's future plugin authoring and runtime contract, independent of the Capsem repo. The prototype should prove the shape of event-driven WASM plugins before we wire anything into the production hook pipeline or user-facing app surface.

The core contract is shared across plugin worlds:

```text
Plugin callback(event_copy, context_copy) -> event_copy
```

Only the event crosses back from WASM. Context is copied into the WASM call as read-only input and is never returned, mutated, or treated as authority.

## Contract

Plugins are callbacks over typed event cards. A callback receives:

- `event`: a mutable event card copy.
- `context`: an immutable host-provided context copy.

The callback returns a modified event. The lower-level WASM boundary validates
the returned event before the host accepts it. Security callbacks are held to a
stricter deterministic/idempotent contract than user-facing UI callbacks, but
the execution engine and envelope are the same.

Target invariant:

```text
same plugin hash + same input event hash + same context hash = same output event hash
```

## Callback Worlds

The prototype should explicitly model two callback worlds:

```ts
SecurityCallback("pii-egress")
  .onHttpRequest((event, context) => event.block("Open-world request after PII"));

UICallback("findings-panel")
  .onPanelRender((event, context) =>
    event
      .setTitle("Findings")
      .setContent({ kind: "findings-table", source: "security.findings" })
  );
```

Security callbacks manipulate `SecurityEvent` cards. UI callbacks manipulate
`UIEvent` cards. They are not the same object, but they share the same
execution engine:

```text
event copy + context copy -> WASM callback -> returned event copy -> boundary validation -> host projection
```

## Security Event Shape

The security event card should include:

```ts
{
  event_id: string;
  trace_id?: string;
  event_family: "http" | "mcp" | "model" | "dns" | "file" | "process" | "credential" | "vm" | "profile" | "conversation";
  event_type: string;
  subject: unknown;
  labels: string[];
  findings: Finding[];
  decision?: Decision;
  mutations: Mutation[];
  redaction_state: "raw" | "redacted" | "summary-only";
}
```

Decision actions must include at least:

```ts
"allow" | "ask" | "block" | "rewrite" | "throttle"
```

Rewrite is represented as declarative mutations, not arbitrary trusted object mutation. The TypeScript authoring API may feel fluent or mutable, but the ABI output should be explicit patches.

## UI Event Shape

The UI event card should use the same event/mutation pattern, but with
user-facing objects instead of enforcement decisions:

```ts
{
  event_id: string;
  event_family: "ui";
  event_type:
    | "ui.panel.render"
    | "ui.command.invoke"
    | "ui.menu.build"
    | "ui.settings.render"
    | "ui.notification.create";
  surface: "dashboard" | "vm_detail" | "settings" | "tray" | "command_palette";
  subject: unknown;
  ui: {
    title?: string;
    content?: unknown;
    actions?: UIAction[];
    menu_items?: UIMenuItem[];
    settings_schema?: unknown;
  };
  mutations: UIMutation[];
}
```

UI callbacks can return declarative UI mutations:

```ts
mutations: [
  { op: "set_title", value: "Findings" },
  { op: "set_content", value: { kind: "table", rows: [] } },
  { op: "add_action", value: { id: "open_finding", label: "Open" } }
]
```

The host owns rendering and validates all UI mutations before applying them.

## Context Shape

Context is copied into WASM and cannot be mutated. Context shape is scoped to
the callback world.

Security context should expose enough information for plugin decisions without
granting ambient authority:

- VM/session/profile identity
- enabled MCP servers
- available MCP tools and annotations
- enabled skills
- profile packs and plugin configuration
- relevant trace/history snapshot
- host capabilities granted to this plugin
- optional ABI handles for explicitly granted host services

UI context should expose enough information for user-facing extension work:

- current app surface
- selected VM/session/profile summary
- enabled plugin settings
- readable app state snapshots granted by manifest capability
- command/menu/view contribution ids
- optional ABI handles for UI-safe host services

Context never crosses back from WASM.

## Host ABI

Plugins have no ambient filesystem, network, process, clock, or credential access.

If a plugin needs host services, the plugin manifest declares capabilities and the host injects limited ABI functions. The first prototype should include HTTPS request and model-query ABI shapes, but both must be capability-gated.

```ts
context.abi.https.request({
  method: "GET" | "POST",
  url: "https://...",
  headers?: Record<string, string>,
  body?: string | Uint8Array,
})
```

Dynamic enforcement plugins may also ask a configured model for a bounded judgment:

```ts
context.abi.model.ask({
  purpose: "dynamic_enforcement",
  prompt: "Classify whether this model output leaks credentials.",
  input: {
    event_id: event.event_id,
    subject_preview: event.subject,
  },
  schema: {
    decision: ["allow", "ask", "block", "rewrite", "throttle"],
    reason: "string",
    confidence: "number",
  },
})
```

The host validates:

- capability is declared in the plugin manifest
- capability is enabled by the install/config layer
- URL is HTTPS
- domain/method/body limits match policy
- result is bounded and redacted as configured
- model purpose is allowed for this plugin
- model/provider/credential use is host-owned, never plugin-supplied
- model input/output token budgets, timeout, and schema limits are enforced
- model result is logged with prompt/input/output hashes and linked to the plugin invocation

## Manifest

Every plugin needs a Chrome-manifest-like manifest so Capsem can enable,
disable, audit, and eventually distribute plugins through a marketplace or
curated registry.

Use a single `capsem_plugin.json` package manifest.

The manifest should include:

- plugin id, name, version, description
- author/vendor identity
- supported ABI version
- exported security callbacks
- exported UI callbacks
- UI contributions
- requested capabilities
- model ABI purpose declarations, if any
- default enablement state
- tags/categories
- permissions summary suitable for UI review
- package/signature metadata placeholder

Sketch:

```json
{
  "manifest_version": 1,
  "id": "acme.pii-guard",
  "name": "PII Guard",
  "version": "0.1.0",
  "description": "Detects and controls PII movement.",
  "abi": "capsem.plugin.v1",
  "entrypoints": {
    "security": {
      "module": "security.wasm",
      "callbacks": [
        "on_http_request",
        "on_model_output"
      ]
    },
    "ui": {
      "module": "ui.wasm",
      "callbacks": [
        "on_panel_render",
        "on_command_invoke"
      ]
    }
  },
  "contributions": {
    "commands": [
      { "id": "pii_guard.show_findings", "title": "Show PII Findings" }
    ],
    "panels": [
      { "id": "pii_guard.findings", "surface": "vm_detail", "title": "PII Findings" }
    ],
    "settings": [
      { "id": "pii_guard.settings", "title": "PII Guard" }
    ]
  },
  "capabilities": {
    "security": ["model.ask"],
    "ui": ["ui.panel", "ui.command", "storage.plugin"]
  }
}
```

Example callbacks:

```text
Security:
on_http_request
on_http_response
on_mcp_request
on_mcp_response
on_model_call
on_model_output
on_tool_call
on_tool_result
on_dns_query
on_file_event
on_vm_start
on_vm_stop
on_profile_loaded

UI:
on_panel_render
on_command_invoke
on_menu_build
on_settings_render
on_notification_create
```

VM lifecycle callbacks like `on_vm_start()` are part of the prototype surface. They receive event/context copies like every other callback and return only an event.

## Runtime Boundary

The WASM boundary is not enough by itself. Plugin execution should live outside
the main Capsem process in an isolated plugin-host subprocess.

Default process model:

```text
capsem-process / service
  -> capsem-plugin-host per VM/session
    -> Wasmtime runtime
      -> plugin component instances
```

The default assumption is one plugin-host process per VM/session. That keeps
plugin state, budgets, logs, and lifecycle naturally scoped to the VM. Inside
that host, each plugin gets its own Wasmtime instance/store with fuel, memory,
timeout, and ABI grants. Later we can add a stricter mode where high-risk or
third-party plugins run in one process per plugin.

The WASM boundary is the language/runtime wall:

1. Host builds event copy and context copy.
2. Capsem sends the invocation to the plugin-host subprocess.
3. Plugin host calls the selected WASM export.
4. WASM returns an event copy only.
5. Plugin host validates event schema, allowed decisions, legal mutations, size limits, and callback-specific rewrite targets.
6. Plugin host returns the validated event and invocation log to Capsem.
7. Capsem maps the final event decision into runtime transport behavior.

The plugin ABI does not return Rust `HookOutcome`. Rust owns that projection.

## Serialization

Because plugins run out-of-process, Capsem and the plugin host must serialize
invocations. The prototype should use a canonical JSON or MessagePack envelope
for:

- event copy
- context copy
- returned event copy
- invocation log

For the WASM component boundary inside the plugin host, we can choose either:

- WIT/component-model records for the long-term ABI, or
- canonical serialized bytes for the first prototype.

The prototype should prefer a stable serialized event envelope so hashing,
replay, and validation are identical across process and WASM boundaries. The
long-term WIT ABI can wrap the same envelope or expose typed records once the
shape settles.

## Observability

Each plugin invocation should emit a structured log record. Later this wires into Capsem telemetry.

The prototype log should track:

- plugin id/version
- callback name
- input event hash
- context hash
- output event hash
- final decision/action
- mutation count
- finding count
- duration
- memory/fuel/timeout status placeholder
- validation status/error

## What Done Looks Like

- A standalone TypeScript package with security event, UI event, context, manifest, mutation, and decision types.
- A small runtime harness that calls plugin callbacks with copied event/context objects.
- Event validation at the simulated plugin-host/WASM boundary.
- UI event validation and declarative UI mutation validation.
- Capability-gated HTTPS ABI shape, with tests proving denied access when not enabled.
- Capability-gated model ABI shape for dynamic enforcement, with tests proving denied access when not enabled.
- Tests for `rewrite`, `throttle`, UI mutation, context immutability, manifest callback declarations, and invocation logging.

## Deferred

- Real WASM compilation/runtime.
- Real Capsem hook integration.
- Real telemetry database integration.
- Plugin signing and marketplace distribution.
- Production HTTP client implementation.
- Stricter one-process-per-plugin isolation mode.
