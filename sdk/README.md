# Capsem gateway SDKs

The Rust gateway contracts produce [OpenAPI](specification/openapi.json), then
generated operations and typed models for [Python](python/README.md),
[TypeScript](typescript/README.md), and [Rust](rust/README.md). The small
`Hypervisor` and `VM` facades share those operations. The web UI uses TypeScript;
the TUI uses Rust. Each client takes an explicit HTTP(S) URL and bearer token.

| Interface | Methods |
| --- | --- |
| Hypervisor | `info`, `list`, `create`, `run`, `purge`, `log`, `update`, `restart` |
| Agent debugging | `debug.panics/triage` |
| Private networks | `networks.create/list/inspect/delete/logs` |
| Profiles and MCP | `profiles.list`, `profiles.mcp(profile).info/servers/default_permission/get`; scoped `server.tools.list/call`, `server.refresh` |
| VM | `info`, `exec`, `persist`, `start`, `stop`, `pause`, `resume`, `delete`, `fork` |
| VM inspection | `log`, `history`, `timeline` |
| VM files | `files.list/read/write` |
| VM networks | `networks.list/attach/detach` |
| VM stats | `stats.summary`, `stats.details` |
| VM container | `container.status` |
| VM ports | `ports.open/list/close` |

Rust accesses the nested resources as methods, for example
`vm.stats().details().await?`. `info` combines versions, profiles and update
state at the hypervisor level, and AI/model/MCP, network and files at the VM
level. `stats.details` returns model usage and typed model, tool, network, DNS,
file, process, audit and credential events, plus captured bodies. It is the
gateway's `/vms/{id}/stats/detail` response.

`stats.details().interactions` is the shared interaction report for all three
SDKs. Its `items` have stable ledger event IDs, parent model event/call IDs,
trace/turn IDs, and a typed `content` variant: request preview, assistant message,
tool call, or tool result. Message blocks distinguish text and reasoning. Tool
calls retain their origin, server, decision and structured arguments; results
retain structured content and the error flag when the ledger recorded it.
Observed MCP JSON-RPC envelopes are retained separately as `request`/`response`;
`arguments` and result `payload` expose the actual tool arguments and result.

Captured payloads distinguish native JSON (including JSON null), text and raw
content. Their status is `complete`, `truncated` or `unknown`. Preview fields
have unknown completeness because the existing ledger does not retain their
original lengths. Truncated bodies remain raw even if their prefix parses as
JSON; malformed JSON and unparsed content retain explicit reasons. Request
previews are not inferred user messages. Assistant items are retained content
fragments, not reconstructed provider message boundaries.

The report contains the latest 200 non-tool-call model items and 200 tool calls,
sorted chronologically, plus the existing bounded body captures. Model tool
calls come from `tool_calls` once; continuation results come from `model_items`.
A call's embedded result uses only its own stored preview. Provider call IDs
are scoped: correlate them with trace/model IDs, never by call ID alone. A null
parent event ID can mean the parent row is no longer retained. Existing stats
event fields remain available while the UI migrates to this shared contract.

Results retain gateway semantics. Guest exec preserves separate `stdout` and
`stderr` lanes. Each field is an `ExecOutput` whose
`encoding` is `utf8` or `base64`, so arbitrary bytes remain exact. A successful
create/start acknowledges launch, and an exec request waits for the guest to become ready.
File access requires a running VM's security ledger. A restart acknowledgement
requires explicit reconnection with new credentials. No mutation is retried.

`run` executes a command in a temporary VM and accepts its own profile, CPU,
memory, environment and guest deadline. `debug.panics` and `debug.triage`
provide typed host diagnostics; triage can include one VM's session ledger. Profile MCP
calls preserve arbitrary JSON arguments and results while discovery and
permissions remain typed.

VM creation accepts a top-level OCI image, command, environment, and typed
`Registry`. The VM remains the workload's private runtime.
The create request returns after the service reports workload readiness;
`vm.container` exposes read-only diagnostic status. `vm.ports.open` creates a
plain host-loopback listener by default; `authenticate=true` selects the
existing browser-authentication flow. The SDK infers the container or VM target.
Port objects can be listed and closed without exposing wire request enums.

Private network operations use authenticated gateway HTTP and immutable network
IDs; VM creation accepts typed objects returned by the network resource.
Creation uses the gateway catalog's default profile when `profile` is omitted:
the client reads `default_profile_id` from `GET /status` once and caches it, so
no profile name is compiled into an SDK. Explicit profile selection accepts a
typed object returned by `profiles.list`, rather than a raw profile ID.
Mounts remain deferred.

The separately installed [`@capsem/mcp`](../mcp/typescript/README.md) package
uses the TypeScript SDK to present these resources to AI agents over stdio. It
requires an explicit gateway URL, bearer token, and HTTP transport timeout; the
native Capsem installer does not install Node.js or download the npm package.

## Oversight

Citadel inventories SDK source, including checked-in generated files. Python
and TypeScript sources share the 300-line ceiling; Rust shares the 1,000-line
ceiling. SDK oversized-file debt is prohibited. Adversarial guard tests verify
that removing SDK roots, coverage, generation checks or CI ownership fails.

The fast gate checks generator drift, Python Ruff/ty/tests/package builds and
TypeScript ESLint/strict Node/browser types/tests/package builds. Rust inherits
workspace warnings-as-errors, Clippy, tests and doctests, with a 97% crate
coverage floor and the shared coverage headroom ratchet. Python and TypeScript
have 90% coverage floors; generated source remains measured. CI publishes
separate SDK coverage components.

## Acceptance fixtures

These Ironbank tests use disposable services and explicit gateway credentials:

- `test_braavos_sdk.py`: all three SDKs, authentication, profile/MCP and host
  diagnostic resources, name resolution,
  stopped workspace files and file-access refusal.
- `test_sdk_live.py`: Python and TypeScript create/exec, exact binary transfer,
  fork isolation, stop/start, pause/resume and deletion on Apple VZ.
- `test_sdk_model.py`: Python inspection of a real VM's model/tool interaction
  against the hermetic upstream, file output, token totals and priced usage.
- `test_sdk_restart.py`: Python and TypeScript acknowledged launchd restart,
  new process identities/token, explicit reconnection and unchanged stopped VM.

The fixture layer owns local binaries, VM assets, profiles and teardown. SDK
drivers communicate only over HTTP. Native macOS acceptance does not establish
Linux supervisor or KVM behavior; those platform owners retain their own gates.
