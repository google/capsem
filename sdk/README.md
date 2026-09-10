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
