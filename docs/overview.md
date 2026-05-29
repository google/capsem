# Capsem Plugin Prototype Overview

## Purpose

This prototype explores a single extension execution system that can support
security automation, direct rules, tools, skills, and user-facing app
extensions without mixing their object models.

The shared contract is:

```text
callback(object_copy, context_copy) -> object_copy
```

The callback receives a copied object and a copied context. Only the object can
cross back. Context is read-only input and is never returned.

Host capabilities that need real Capsem authority are exposed as explicit ABI
calls. For example, a file-create plugin can read `.git/HEAD` and `.git/config`
through ABI-backed `context.fs.read(...)`, fetch GitHub repository statistics
through ABI-backed `context.fetch(...)`, and emit a UI block through
`context.ui.emit(...)`, but it still returns the `FileCreate` object as its
callback result.

## Extension Package Model

The installable unit is an extension package, similar in spirit to a VS Code
extension. A package can contribute several routed unit types:

```text
capsem extension package
  -> Plugin()  -> TypeScript authoring compiled to WASM/runtime artifact
  -> Rule()    -> TypeScript authoring compiled to IR/CEL policy bundle
  -> Tool()    -> callable capability contract exposed to agents/users
  -> Skill()   -> agent workflow/instruction package
  -> UI()      -> user-facing extension surface
  -> Assets    -> schemas, fixtures, icons, docs, tests
```

Plugin and Rule are not competing systems. They share package identity,
manifest validation, object/context schemas, tests, and trace semantics. The
compiler routes them differently:

```text
Plugin() -> runtime artifact loaded by plugin host
Rule()   -> checked IR/CEL bundle evaluated by policy engine
Tool()   -> tool registry entry with capability gates
Skill()  -> skill index entry for agents
UI()     -> sandboxed UI contribution
```

The first product-shaped AssemblyScript spike is a git context plugin:

```ts
const plugin = Plugin("capsem.git-context").on_file_create((file_event, context) => {
  if (file_event.path != ".git" && !file_event.path.endsWith("/.git")) {
    return file_event;
  }

  const head = context.fs.read(file_event.path + "/HEAD");
  const config = context.fs.read(file_event.path + "/config");
  const github_api_url = github_repo_api_url(config);
  const github_stats = github_api_url.length > 0
    ? parse_github_stats(context.fetch(github_api_url))
    : "github stats unavailable";

  context.ui.emit(new UiMutation(
    "workspace.context",
    "upsert_block",
    new UiBlock("git-context-card", "git-context", "Git", head, github_stats),
  ));
  return file_event;
});
```

`context.fs`, `context.fetch`, and `context.ui` are ABI-backed SDK surfaces.
`fetch` is the Chrome-extension-style word on purpose: the manifest declares
the capability/host policy, the plugin calls `context.fetch(url)`, and the host
brokers the request under trace. The plugin does not get ambient filesystem,
network, or UI access, and the file object does not own UI methods.

For production, TypeScript source is the audit/rebuild artifact. The runtime
artifact is the execution artifact. Intermediate portable WASM is disposable
unless retained for debugging/provenance.

```text
source package
  -> inspect + validate manifest/contributions
  -> compile temporary component
  -> precompile runtime artifact, e.g. <blake3>.cwasm
  -> smoke load/run
  -> discard temporary WASM
  -> keep source package + internal manifest + runtime artifact
```

## Authoring Shape

The authoring surface should stay as close to familiar JavaScript and Chrome
extension concepts as possible. That is not aesthetic; it is an adoption and
correctness constraint. Human authors and agent authors need strong analogies:
manifest-declared capabilities, `fetch`, callback hooks, explicit host
permissions, and narrow injected APIs are easier to reason about than a
Capsem-specific vocabulary invented too early.

Callbacks should not be named manually in user code. The hook name comes from
the method being implemented.

Preferred shape:

```ts
export default Plugin({
  security: {
    onHttpRequest(request, context) {
      if (context.trace.labels.includes("pii_access")) {
        return request.block("Open-world request after PII access");
      }

      return request;
    },

    onModelOutput(output, context) {
      return output
        .rewrite("SSN-like value found")
        .replaceRegex("content.text", ssnPattern, "[REDACTED]");
    },
  },

  ui: {
    onPanelRender(panel, context) {
      return panel
        .setTitle("Findings")
        .setContent({ kind: "findings-table", source: "security.findings" });
    },
  },
});
```

Avoid generic names like `event`. The object name should describe the thing
being handled:

- `request` for HTTP/MCP/model requests
- `response` for HTTP/MCP responses
- `call` for tool/model calls
- `output` for model output
- `query` for DNS queries
- `file` for file activity
- `vm` for VM lifecycle objects
- `panel`, `command`, `menu`, `settings`, or `notification` for UI objects

## Object Families

The engine is shared, but the objects are not.

Security objects carry enforcement state and are used by `Plugin()` callbacks
and `Rule()` evaluation:

```text
HttpRequest
HttpResponse
McpRequest
McpResponse
McpToolList
McpToolCall
McpToolResult
ModelCall
ModelOutput
ToolCall
ToolResult
DnsQuery
NetworkConnection
FileActivity
FileRead
FileWrite
ProcessExec
ClipboardRead
ClipboardWrite
VmStart
VmReady
VmStop
VmSuspend
VmResume
ProfileLoaded
ProfileChanged
PolicyBundleLoaded
SecretAccess
CredentialUse
BrowserNavigation
BrowserDownload
```

UI objects carry user-facing app mutations:

```text
PanelRender
CommandInvocation
MenuBuild
SettingsRender
NotificationCreate
StatusBadge
ToolbarAction
InspectorView
TraceView
FindingDetail
QuickPick
```

Tool objects describe callable surfaces contributed by an extension:

```text
ToolDefinition
ToolInvocation
ToolResult
ToolAuthorization
ToolBudget
```

Skill objects describe agent-facing workflows and instructions:

```text
SkillDefinition
SkillActivation
SkillContext
SkillResult
```

Rule objects describe direct policy evaluation units:

```text
RuleBundle
RuleCallback
RuleMatch
RuleDecision
RulePatchSet
RuleActionRequest
```

Security objects and UI objects should share envelope mechanics, hashing,
serialization, validation, and logging, but they should not pretend to be the
same data type.

## Core Object Envelope

Every object family should fit a common envelope:

```json
{
  "kind": "ModelOutput",
  "id": "object-id",
  "trace_id": "trace-id",
  "session_id": "session-id",
  "profile_id": "profile-id",
  "subject": {},
  "labels": [],
  "findings": [],
  "decision": {
    "verdict": "allow",
    "reasons": []
  },
  "patches": [],
  "actions": []
}
```

The envelope is stable; `subject` is typed by `kind`.

## Security Objects

Security objects can carry:

- identity: object id, trace id, session/profile ids
- subject data: request, response, model output, tool call, etc.
- labels
- findings
- decision
- declarative mutations
- redaction state

Security decisions include:

```text
allow
ask
block
rewrite
throttle
```

Rule evaluation should return the same decision shape as plugin callbacks. CEL
stays pure: it can produce decisions, patches, and action requests, but it does
not perform host effects directly.

Rewrite is represented as declarative mutations. The authoring API may feel
fluent, but the returned object must contain validated mutation records rather
than trusted arbitrary side effects.

Example internal mutation:

```json
{
  "op": "replace_regex",
  "path": "content.text",
  "pattern": "[0-9]{3}-[0-9]{2}-[0-9]{4}",
  "replacement": "[REDACTED]"
}
```

Rust applies the actual wire/body/header mutation after validating that the
mutation is legal for the object type.

## UI Objects

UI callbacks use the same callback envelope, but manipulate UI-specific
objects. They return declarative UI mutations such as:

```json
{ "op": "set_title", "value": "Findings" }
{ "op": "set_content", "value": { "kind": "table", "rows": [] } }
{ "op": "add_action", "value": { "id": "open_finding", "label": "Open" } }
```

The host owns rendering. UI plugins describe what they want to contribute, and
Capsem validates and renders it.

## Context

Context is copied into the callback and cannot be mutated. It exists so plugins
can make decisions without gaining ambient authority.

Security context can expose:

- VM/session/profile identity
- enabled MCP servers
- available MCP tools and annotations
- enabled skills
- profile packs and plugin configuration
- relevant trace/history snapshot
- granted host capabilities
- ABI handles for explicitly granted services

UI context can expose:

- current app surface
- selected VM/session/profile summary
- enabled plugin settings
- allowed app state snapshots
- contribution ids
- ABI handles for UI-safe services

Context never crosses back from WASM.

## Host ABI

Plugins have no ambient filesystem, network, process, clock, credential, or
model access.

Host services are explicit ABI capabilities declared in the manifest and
enabled by the host.

Planned ABI surfaces:

```ts
context.abi.https.request(...)
context.abi.model.ask(...)
context.abi.storage.plugin(...)
```

The host owns credentials, provider selection, network policy, budgets,
timeouts, schemas, and logging.

## Isolation

WASM should not run in the main Capsem process.

Default shape:

```text
capsem-process / service
  -> capsem-plugin-host per VM/session
    -> Wasmtime runtime
      -> plugin component instances
```

The default assumption is one plugin-host subprocess per VM/session. Each
plugin gets its own Wasmtime instance/store with fuel, memory, timeout, and
capability grants.

Later, high-risk plugins can move to one process per plugin without changing
the authoring model.

## Serialization

Because the plugin host is out-of-process, objects, contexts, returned objects,
and invocation logs must serialize across the boundary.

The prototype should use a stable serialized envelope first, likely canonical
JSON or deterministic MessagePack. The long-term WASM component ABI can wrap
that envelope or expose typed WIT records after the object shapes settle.

Stable serialization matters for:

- hashing
- replay
- deterministic tests
- audit records
- plugin marketplace validation

## Manifest

Use a Chrome/VS Code-style package manifest, tentatively `capsem_plugin.json`.

The manifest should describe:

- extension identity
- version
- ABI version
- contributed plugins
- contributed rules
- contributed tools
- contributed skills
- contributed UI surfaces
- implemented callback methods per plugin
- requested capabilities
- default enablement
- permissions summary
- signing/package metadata

Sketch:

```json
{
  "manifest_version": 1,
  "id": "acme.pii_guard",
  "name": "PII Guard",
  "version": "0.1.0",
  "abi": "capsem.plugin.v1",
  "contributes": {
    "plugins": [
      {
        "id": "acme.pii_guard.security",
        "source": "src/security.ts",
        "callbacks": ["onHttpRequest", "onModelOutput"]
      }
    ],
    "rules": ["rules/pii.rules.json"],
    "tools": ["tools/redact.tool.json"],
    "skills": ["skills/pii-review/SKILL.md"],
    "ui": ["ui/findings-panel.json"]
  },
  "capabilities": {
    "security": ["model.ask"],
    "ui": ["ui.panel", "ui.command", "storage.plugin"]
  }
}
```

The internal manifest is Capsem-owned. It records source hashes, contribution
metadata, compiler/runtime versions, runtime artifact hashes, smoke-test
results, and trace policy.

## Invocation Logs

Every callback invocation should produce a structured log record. Later this
wires into Capsem telemetry.

Track:

- plugin id/version
- object type
- callback method
- input object hash
- context hash
- output object hash
- final decision, for security objects
- mutation count
- finding count
- duration
- memory/fuel/timeout status placeholder
- validation status/error

## Open Questions

- Which concrete security plugins should we prototype first?
- Which concrete UI plugins should we prototype first?
- Is canonical JSON enough for the first boundary, or should we start with
  deterministic MessagePack?
- Should UI and security entrypoints live in separate WASM modules by default?
- What is the minimal useful model ABI schema for dynamic enforcement?
