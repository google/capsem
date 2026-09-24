# Porting Map: Prototype To Mainline

## Current Lock State

This document is a porting map, not permission to port. Mainline integration
remains blocked until the contract/catalogue/UI lock is complete and reviewed.

The isolated lane has passed these proof gates:

- `npm run e2e:chat`: browser E2E for create, chart/diagram hydration,
  exact-element feedback, mutation, render-error UI, theme/dark rendering,
  reconnect, reload, and resolve.
- `npm run inspect:workspace`: live telemetry plus durable SQLite
  record/checkpoint inspection for create, comment, style patch, title patch,
  render error, resolve, and checkpoint.
- `npm run bench:workspace`: normalized projection, snapshot, mutation,
  checkpoint, websocket fanout, restart, and browser render benchmark.

Still blocked:

- Full component catalogue lock.
- Mainline session telemetry wiring.
- Mainline service lifecycle wiring.
- Generated schema/type source of truth for Rust, TS, and Python.
- Production topology ids for every shipped component.

## Prototype Capabilities To Preserve

1. Workspace stream and projection
   - Append-only records.
   - Rust reducer.
   - Checkpoint and replay.
   - Keyed Svelte render maps.

2. UI element foundation
   - Web component island boundary.
   - Shadow DOM for rendered artifacts.
   - Composed events back to Svelte shell.
   - Render error handling.

3. Annotation and comments
   - Floating Comment tool.
   - Exact element annotation.
   - Floating popover with Comment/Cancel.
   - Spatial marker.
   - Visible task list.
   - Resolve lifecycle.

4. Mutations
   - Title patch.
   - Selector-scoped style patch.
   - Light DOM and Shadow DOM targets.
   - Whitelisted style properties.

5. Toolability
   - `ui.info` style read model.
   - Snapshot/stream API.
   - Comment/resolve/mutate routes.

6. Rich artifacts
   - Generated image cards.
   - Native chart/table/diagram/slide seeds.
   - Artifact provenance and render metadata.

## Mainline Owner Inventory

The isolated prototype does not modify mainline. These are the likely owner
areas discovered from the current Capsem tree and must be confirmed again at
port time.

| Concern | Mainline Area | Prototype Source | Porting Notes |
| --- | --- | --- | --- |
| Rust record/projection runtime | New or imported `capsem-ui-runtime` crate in main workspace | `crates/capsem-ui-runtime/**`, `crates/capsem-ui-catalog/src/workspace.rs` | Move ownership out of catalogue re-exports before mainline. |
| UI shell and renderers | `frontend/src/**` | `ui-preview/src/ChatPage.svelte`, `ui-preview/src/elements/**` | Port state machine and component foundation into existing Svelte/Preline app patterns. |
| Service API/session lifecycle | `crates/capsem-service/src/**` | `crates/capsem-plugin-server/src/main.rs` | Replace prototype Axum routes with service-owned routes or command handlers. |
| Local MCP/tool exposure | `crates/capsem-mcp/**`, `crates/capsem-mcp-aggregator/**`, `crates/capsem-mcp-builtin/**` | `/native/mcp/tools`, `python/capsem_native_client` | Tools must call Rust runtime; no direct frontend mutation shortcuts. |
| Session telemetry/audit | `crates/capsem-core/src/**`, session DB/debug tooling | `NativeTelemetryEvent`, `scripts/inspect-workspace-session.ts` | Persist the same event fields in real session telemetry, not the prototype in-memory vector. |
| Model/settings policy | `config/settings-schema.json`, service settings loaders, `crates/capsem-ai` | `crates/capsem-ai/src/lib.rs` | No code defaults; provider/model/credential policy comes from settings. |
| Frontend tests | frontend test suite and Playwright harness | `test/*.test.ts`, `scripts/e2e-chat.ts` | Preserve exact-element annotation, replay, render-error, and theme/dark checks. |
| Bench/session inspection | benchmark and session-debug tooling | `scripts/bench-workspace.ts`, `scripts/inspect-workspace-session.ts` | Promote into Capsem benchmark/session-inspection commands after integration. |

## Prototype Files

| Prototype File | Production Role |
| --- | --- |
| `crates/capsem-ui-catalog/src/workspace.rs` | Seed for `capsem-ui-runtime` crate. |
| `crates/capsem-ui-runtime/src/lib.rs` | Current runtime dependency seam; re-exports workspace/editable state until artifact ownership is split. |
| `crates/capsem-ui-runtime/tests/runtime_boundary.rs` | Proof that the runtime crate builds and projects/checkpoints records without server code. |
| `crates/capsem-ui-catalog/tests/workspace.rs` | Seed for reducer/contract tests. |
| `crates/capsem-plugin-server/src/main.rs` | Reference only; split route/tool glue from reusable runtime. |
| `ui-preview/src/nativeDeck.ts` | Seed for generated TS types/parsers; should be generated from schema where possible. |
| `ui-preview/src/ChatPage.svelte` | Reference for UX and state machine; port into mainline UI shell, not copied wholesale. |
| `ui-preview/src/elements/capsem-elt.ts` | Seed for component island foundation. |
| `ui-preview/src/elements/capsem-artifacts.ts` | Seed for artifact renderers and annotation event pattern. |
| `test/native-workspace.test.ts` | Seed for TS parser/contract tests. |
| `test/capsem-element.test.ts` | Seed for renderer behavior tests. |
| `schemas/a2ui/v0_9/**` | Baseline A2UI Basic schema source for conformance. |
| `schemas/capsem-ui/**` | Seed for Capsem extension catalogue. |

## Production Replacements

- Move the workspace implementation from transitional re-export into
  `capsem-ui-runtime` once artifact types are separated from catalogue helpers.
- Replace ad hoc TS type definitions with generated Rust/TS types from a shared schema.
- Replace in-memory `NativeWorkspace` with persistent workspace storage.
- Replace numeric `data-capsem-node` ids with stable component topology ids.
- Replace raw style-patch-first behavior with typed mutations where available.
- Replace prototype server routes with mainline service/local tool/MCP routes.
- Replace prototype chat shell with mainline Capsem UI layout and navigation.
- Replace implicit A2UI subset assumptions with an explicit coverage matrix:
  schema/type, high-level helper, renderer, topology, mutation, and tests.
- Replace incomplete local route naming with the frozen MCP/tool hierarchy from
  `crdt-a2ui-mcp.md`.

## State Substrate Porting Caution

T0A selected the Capsem record envelope plus Loro editable state. Do not reopen
Automerge/Yrs/Yjs or A2UI-native streams unless a new fixture proves the record
spine cannot support the product requirement.

The production port should keep:

- Authoritative append-only records.
- Rust projection/checkpoint/replay.
- Loro only for editable rich-object state where collaborative/local editing
  justifies it.
- A2UI as a render/interchange vocabulary, not the authoritative session log.

## Artifact Porting Caution

Do not split slides, spreadsheets, generated media, charts, diagrams, and
timelines into separate engines. They should all enter as typed workspace
artifacts and render through the same projection/topology/mutation path.

## Tool Contract To Preserve

The first mainline tool names should map to the prototype descriptors. Adapter
names may follow Capsem MCP conventions, but the semantic hierarchy should stay:

| Tool | Required Behavior |
| --- | --- |
| `local.ui.info` | Return topology, elements, tasks, selected target, and mutation affordances. |
| `local.ui.comment` | Attach feedback to an exact topology/annotation target. |
| `local.ui.resolve` | Resolve a task through a response record. |
| `local.ui.mutate` | Apply typed title/text/style mutations through the Rust allowlist. |
| `local.workspace.snapshot` | Return checkpoint plus projection state. |
| `local.workspace.stream` | Replay from `lastSeq` and stream subsequent records. |
| `local.workspace.checkpoint` | Compact records into a replay-safe checkpoint. |
| `local.generate.*` | Create generated media/text artifacts with usage/cost metadata. |
| `local.data.*` | Back sheet/spreadsheet/table/chart preparation through the local data lane. |

No tool should write Svelte state directly. Tools append records or request
runtime reads.

## Telemetry Contract To Preserve

Mainline telemetry must contain at least the fields proven by
`npm run inspect:workspace`:

- operation: `workspace.create`, `workspace.request`, `workspace.patch`,
  `workspace.respond`, `render.error`, and later denial/model/tool operations.
- seq, record id, timestamp, role, principal, title, content type, verb,
  status, target, duration.
- task id when a task is created or resolved.
- mutation summary for artifact, title/text/style patch, tool call, and error
  records.
- renderer/source/phase/message fields for render failures.
- durable session/workspace identity from mainline, not prototype constants.

The prototype inspection proves the shape, not persistence of telemetry across
restart. Mainline must persist or export it through the existing session
telemetry path.

## Testing Gates To Port

| Gate | Prototype Command | Mainline Equivalent |
| --- | --- | --- |
| Rust contract | `cargo test -p capsem-ui-catalog -p capsem-ui-runtime -p capsem-plugin-server` | Runtime/service crate tests plus generated schema snapshots. |
| TS/browser contract | `npm run typecheck && npm test` | Frontend typecheck and component/parser tests. |
| Python/tool rehearsal | `PYTHONPATH=python python3 -m unittest discover -s python/tests` | MCP/client tool tests using mainline tool adapters. |
| Browser E2E | `npm run e2e:chat` | Mainline Playwright path through real app shell. |
| Session inspection | `npm run inspect:workspace` | Real session DB/telemetry inspection after app run. |
| Performance | `npm run bench:workspace` | Mainline benchmark without prototype process-start noise. |

## First Integration Proof

The first mainline proof should do exactly this:

1. Create a card through a local tool/API call.
2. Open the mainline UI.
3. Comment on the card title.
4. Verify the task list shows the pending comment.
5. Use the local tool/API to inspect topology.
6. Apply a title or color mutation through the runtime.
7. Verify the UI changes through stream/replay.
8. Resolve the comment.
9. Restart the app/service.
10. Verify the card, mutation, and resolved task are still present.
11. Inspect telemetry for the full record chain.
12. Verify every emitted A2UI component has schema validation, renderer coverage,
    and a topology id.
13. Verify the selected state substrate can replay or restore the same projected
    UI after restart.
14. Verify one rich artifact, preferably chart or generated image, follows the
    same record/projection/topology path.

## Mainline PR Notes

- Keep the initial PR behind a feature flag if the UI shell is not ready.
- Do not mix plugin/WASM runtime changes into this PR.
- Do not land generated media provider changes unless needed for the integration proof.
- Review security implications of style mutation and selector/topology exposure.
- Keep the first PR small enough to review: runtime crate, one UI surface, one
  service/tool path, one persistence path, one telemetry proof.
