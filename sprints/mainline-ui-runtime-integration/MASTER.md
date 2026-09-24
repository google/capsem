# Mainline UI Runtime Integration

## Goal

Deliver the prototype workspace/UI runtime into Capsem mainline as a production feature that can be used by the app UI, local MCP tools, model/tool workflows, and later plugin output.

The feature is not "chat comments" alone. The production feature is a shared UI/workspace execution model:

- Rust owns records, projection, persistence, replay, checkpoints, provenance, and validation.
- Svelte/Preline renders typed projections and sends user actions back as records.
- Comments, mutations, generated artifacts, component tasks, cards, tables, charts, and future plugin UI all use the same lane.
- Tools and agents use a stable local tool/API surface, not private route shortcuts.

## Mainline Lock

Mainline integration is blocked until the isolated prototype is a complete,
locked, tested, and benchmarked feature. The current branch is still a design
and proof lane, not a merge-ready feature branch. Do not start mainline porting
until all of these gates pass in the isolated lane:

- Contract lock: Rust/TS/Python schemas, record envelope, topology, tasks,
  mutation verbs, local tools, generated media, artifact taxonomy, and renderer
  coverage are frozen.
- Functional lock: the chat UI can create, inspect, comment, mutate, resolve,
  replay, checkpoint, restore, and reconnect through the same runtime lane.
- Catalogue lock: shipped components have schema, A2UI/Capsem mapping,
  Preline/Svelte renderer, topology ids, mutation support, and failure tests.
- Export lock: slides, sheets, charts, diagrams, and generated media can be
  exported through typed MCP/local tools into user-downloadable artifacts. A
  preview without a usable export file is not complete.
- Telemetry/security lock: records, render errors, denied mutations, model/tool
  calls, costs, timings, and decisions are auditable.
- E2E lock: browser tests prove live editing, exact-element feedback, refresh,
  reconnect, persistence, dark/theme rendering, and task resolution.
- Benchmark lock: projection, replay, checkpoint restore, stream fanout,
  mutation latency, and component rendering have normalized measurements.

## Status

| Sprint | Status | Purpose | Exit Gate |
| --- | --- | --- | --- |
| T0 Contract Freeze | Inventory Seeded | Freeze record, topology, task, mutation, and component contracts from prototype. | Mainline owner inventory and porting map exist; Rust + TS schema snapshots still need review and lock. |
| T0A Protocol Bake-Off | Conformance Seeded | Test competing protocol shapes before building the runtime; decide state, A2UI, artifact, and MCP contracts. | Loro Rust/browser API audit, contract matrices, and Pack 01 conformance tests pass before T1 implementation. |
| T1 Crate Extraction | Reducer Seeded | Extract `capsem-ui-catalog`/workspace runtime into a portable crate shape for mainline. | `capsem-ui-runtime` builds standalone, prototype server imports workspace runtime through it, and comment tasks replay through projection/checkpoint state. |
| T2 Persistence | Prototype Wired | Move workspace records, comments, tasks, checkpoints, and artifacts from memory to DB/session storage. | Restart preserves UI state and tasks through runtime storage and optional prototype service wiring; mainline wiring blocked by Mainline Lock. |
| T3 Main UI Port | Blocked | Port chat shell, component renderer, annotation overlay, task list, theme behavior into the real Capsem UI. | May start only after Mainline Lock passes. |
| T4 Local Tool/MCP Surface | Schema-Conformant | Expose stable `local.ui.*`, `local.workspace.*`, `local.generate.*` tools over Capsem’s local tool layer. | Prototype descriptors/helpers pass, descriptor status must match the matrix, implemented generation/artifact tools expose request-shaped schemas, and representative schema accept/reject tests pass; real mainline MCP wiring remains blocked by Mainline Lock. |
| T5 Component Catalogue | Timeline Renderer Covered | Harden Preline/A2UI component catalogue and topology ids for production rendering. | Pack 01, rich artifact schemas, Plotly chart API, Mermaid/timeline API, generated-media matrices, native artifact validators, Rust-generated schema snapshot, generated TS/Python bindings, parser tests, source-level Plotly/timeline renderer drift coverage, browser chart/timeline hydration, fit, and double-axis assertions pass; broader visual topology coverage still pending. |
| T6 Telemetry/Security | Prototype Covered | Audit every user/model/plugin mutation and enforce allowed mutation/style surface. | Prototype workspace, render-error, and style allowlist telemetry are covered; full renderer/schema coverage and mainline session telemetry remain pending. |
| T7 Prototype Lock Gates | Runtime Topology Covered | Run isolated functional, adversarial, browser, telemetry, and benchmark gates before any mainline port. | Chat browser E2E covers stable row/image-caption/slide-text topology annotations plus style/text mutations, runtime sheet/chart/diagram/image/timeline/slide/deck topology roles, restart/reload, workspace runtime benchmark, isolated session inspection, and Rust/TS topology registry drift gates pass; broader editor catalogue lock still blocks mainline porting. |

## Non-Negotiables

- No direct DOM cheating for agent changes. Agents emit typed workspace records.
- No ephemeral-only comments. User comments are persistent records with task state.
- No brittle anonymous selectors as the production identity model. DOM selectors are a fallback/debug view; component topology ids are the contract.
- No unbounded CSS patching. Mutations are typed and whitelisted.
- No hidden model/provider defaults. Model policy and credentials come from settings.
- No plugin/WASM coupling in this sprint. This sprint makes the UI/workspace runtime landable ahead of full plugin execution.
- No partial A2UI claims. Every shipped `ui.*` helper must lower to valid A2UI or a documented Capsem extension, and every emitted component must have a checked renderer.
- No local-tool folklore. The MCP/tool surface must be named, typed, tested, and mapped to the Rust runtime before integration.
- No comment/task folklore. Change requests are projected `WorkspaceTask` state with replayable open/resolve lifecycle.
- No fake document story. Slide decks, spreadsheets, charts, diagrams, images,
  and PDFs must have a real export path through the same typed runtime/tool
  lane, even when the implementation delegates to packaged tools in the Capsem
  image.
- No JS UX afterthought. The browser/editor surface is a first-class contract:
  users must be able to inspect, edit, comment, and export the same artifact
  model that agents manipulate.

## Prototype Sources

- `crates/capsem-ui-catalog/src/workspace.rs`
- `crates/capsem-ui-runtime/src/lib.rs`
- `crates/capsem-ui-catalog/tests/workspace.rs`
- `crates/capsem-plugin-server/src/main.rs`
- `ui-preview/src/ChatPage.svelte`
- `ui-preview/src/elements/capsem-elt.ts`
- `ui-preview/src/elements/capsem-artifacts.ts`
- `ui-preview/src/nativeDeck.ts`
- `test/native-workspace.test.ts`
- `test/capsem-element.test.ts`
- `schemas/a2ui/v0_9/**`
- `schemas/capsem-ui/**`

## Mainline Delivery Shape

This is the eventual delivery shape after the Mainline Lock passes. It is not
permission to port early.

1. `capsem-ui-runtime` Rust crate: records, projection, tasks, mutations, persistence adapters, validation.
2. `capsem-ui` frontend package/module: Svelte components, Preline renderers, annotation overlay, task ledger, topology-aware component shells.
3. Local tool surface: stable MCP/tool routes that call the Rust runtime and never bypass projection.

## Lock Order

Do not spend the next sprint on broad catalogue expansion or mainline porting
until the runtime proof is locked. The current priority order is:

1. Mutation semantics: topology-addressed edits, comments, task state,
   checkpoints, replay, resume, reconciliation, and live replacement.
2. Renderer/error hardening: strict renderer drift gates, stable topology,
   render-error reporting, and no silent fallback when a declared component has
   no renderer.
3. Export/runtime tool lock: MCP/local tools can build a slide deck one slide
   at a time, edit it, and export a downloadable deck; the same pattern applies
   to spreadsheets plus charts and PDF/SVG/PNG outputs.
4. Persistence/telemetry lock: durable records, checkpoints, task lifecycle,
   model/tool usage, mutation/export decisions, file outputs, and audit timing.
5. Performance refresh: normalized latency/QPS for projection, mutation,
   checkpoint, replay, stream fanout, and renderer hydration under realistic
   artifact mixes, including export jobs where applicable.
6. Broader editor catalogue coverage after the spine is locked.
7. Mainline port plan only after the isolated feature is locked, tested, and
   benchmarked.

## MCP Acceptance Gate

The end-to-end proof is agent-driven, not hand-assembled. Using only the local
MCP/tool surface, an agent must be able to:

1. Create a slide deck by dictating one slide at a time.
2. Add images, tables, charts, diagrams, and timeline blocks through typed
   artifact calls.
3. Inspect topology and comments, apply targeted edits, and replay/reconnect
   without losing state.
4. Export a PowerPoint-compatible deck file that the user can download/upload.
5. Create a spreadsheet with data and embedded/exported charts, then export an
   XLS/XLSX-compatible workbook.
6. Export charts/diagrams/media as PNG/SVG and produce a credible PDF path for
   deck or report output.

The implementation should prefer packaged tools inside the Capsem image for
office/PDF document conversion: LibreOffice/OpenOffice, Python document
libraries, Chromium print, Pandoc, or similar auditable tooling. Plotly and
Mermaid are different: they are live UI renderer contracts, so the browser
renderer is authoritative and export must come from that renderer path or a
parity-checked headless browser path. Rust should be the typed control plane,
validator, job runner, storage boundary, and telemetry wall. Do not implement
office/PDF engines in Rust or JavaScript unless that specific implementation is
clearly the best tool for the job.

## Active Design Questions

- Which protocol should carry UI/artifact records, replay, comments, mutations, checkpoints, and tool calls?
- Should workspace replay/checkpoint remain an authoritative Capsem append log, or should the sprint adopt a CRDT substrate such as Yrs/Yjs, Automerge, or Loro for offline/collaborative state?
- How do Loro-backed editable UI nodes project into A2UI Basic or Capsem extension render specs without creating a second editable state path?
- Which MCP tools are part of the production contract, and which remain internal test helpers?

## Release Holds

- Hold all mainline porting until the isolated prototype is functionally locked,
  tested, benchmarked, and reviewed against the Mainline Lock gates.
- Hold release until persistence, replay, and comment task lifecycle survive app restart.
- Hold release until browser E2E covers user-created comments, agent-applied mutation, resolve lifecycle, reconnect/replay, and dark/theme rendering.
- Hold release until component topology ids replace numeric DOM-node ids for production components.
- Hold release until telemetry/audit includes principal, target, mutation type, source request, timing, status, and failure reason.
- Hold implementation until T0A tests protocol alternatives and records the selected contract.
