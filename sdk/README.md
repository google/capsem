# Capsem gateway SDKs

The Rust gateway contracts produce [OpenAPI](specification/openapi.json), then
generated operations and typed models for [Python](python/README.md),
[TypeScript](typescript/README.md), and [Rust](rust/README.md). The small
`Hypervisor` and `VM` facades share those operations. The web UI uses TypeScript;
the TUI uses Rust. Each client takes an explicit HTTP(S) URL and bearer token.

| Interface | Methods |
| --- | --- |
| Hypervisor | `info`, `list`, `create`, `log`, `update`, `restart` |
| VM | `info`, `exec`, `start`, `stop`, `pause`, `resume`, `delete`, `fork` |
| VM inspection | `list`, `log`, `history`, `timeline`, `changes` |
| VM snapshots | `snapshots.list`, `snapshots.status` |
| VM stats | `stats.summary`, `stats.details` |
| VM copy | Python/Rust `from_vm` and `to_vm`; TypeScript `fromVm` and `toVm` |

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

Results retain gateway semantics. In particular, guest exec currently combines
stdout and stderr into `stdout`; `stderr` is empty. A successful create/start
acknowledges launch, and an exec request waits for the guest to become ready.
File copy requires a running VM's security ledger. A restart acknowledgement
requires explicit reconnection with new credentials. No mutation is retried.

Snapshot creation/restoration, mounts, port exposure and subnet configuration
remain deferred. The SDK does not itself enable remote or container networking.

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

- `test_sdk_gateway.py`: all three SDKs, authentication, name resolution,
  stopped workspace files, snapshot changes and copy refusal.
- `test_sdk_live.py`: Python and TypeScript create/exec, exact binary transfer,
  fork isolation, stop/start, pause/resume and deletion on Apple VZ.
- `test_sdk_model.py`: Python inspection of a real VM's model/tool interaction
  against the hermetic upstream, file output, token totals and priced usage.
- `test_sdk_restart.py`: Python and TypeScript acknowledged launchd restart,
  new process identities/token, explicit reconnection and unchanged stopped VM.

The fixture layer owns local binaries, VM assets, profiles and teardown. SDK
drivers communicate only over HTTP. Native macOS acceptance does not establish
Linux supervisor or KVM behavior; those platform owners retain their own gates.
