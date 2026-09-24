# Sprint: mainline-ui-runtime-integration

## Tasks

- [x] Create formal integration sprint docs.
- [x] T0 inventory mainline UI/service/MCP/settings/telemetry paths.
- [ ] T0 freeze record/projection/task/mutation/component contracts.
- [x] T0 write porting map from prototype files to mainline files.
- [x] T0A test protocol alternatives before building.
- [x] T0A compare Capsem record protocol, A2UI-native stream, A2UI plus Capsem envelope, CRDT document protocol, and MCP event log.
- [x] T0A compare CRDT candidates Yrs/Yjs, Automerge, and Loro only as part of protocol bake-off.
- [x] T0A encode the full fixture across UI, chart, diagram, spreadsheet, slide deck, website/page/form, generated media, comment, mutation, resolve, checkpoint, and replay.
- [x] T0A build A2UI Basic coverage matrix.
- [x] T0A freeze Pack 01/Pack 02/Capsem-extension component boundaries.
- [x] T0A freeze MCP/local tool hierarchy and deferred tools.
- [x] T0A freeze rich artifact taxonomy for slides, sheets, websites/forms, media, charts, diagrams, and timelines.
- [x] T0A add conformance tests for helper-to-wire-to-render-to-topology.
- [x] T0A write `selected-protocol.md` with winner, rejected alternatives, and proof matrix.
- [x] T0A run Loro API audit fixture in Rust and browser before T1 implementation.
- [x] T0A enforce one-code-path rule: no Automerge/Yrs/Yjs dependencies unless protocol decision is reopened.
- [x] T1 extract reusable Rust UI runtime crate.
- [x] T1 make comment/change requests durable projection tasks with open/resolve/checkpoint behavior.
- [x] T1 add reducer tests for comments, resolution, style/text/title mutations, and checkpointing.
- [x] T1 add adversarial reducer tests for malformed task actions and missing targets.
- [x] T2 add runtime SQLite persistence adapter and DB/session schema.
- [x] T2 prove runtime restart restores records, tasks, projection, checkpoints, and tail replay.
- [x] T2 wire persistence adapter into prototype service lifecycle behind `CAPSEM_NATIVE_WORKSPACE_DB`.
- [ ] LOCK contract, catalogue, UI flows, telemetry, E2E, and benchmarks before any mainline port.
- [ ] BLOCKED T2 wire persistence adapter into mainline service/session lifecycle after lock gates.
- [ ] BLOCKED T3 port Svelte component foundation and Preline renderers into main UI after lock gates.
- [ ] BLOCKED T3 port annotation overlay, comment popover, and task ledger after lock gates.
- [ ] BLOCKED T3 replace numeric DOM-node ids with component topology ids for production components after lock gates.
- [x] T4 expose prototype local tool/MCP descriptor surface for UI info, comment, resolve, mutate, snapshot, and stream.
- [x] T4 prototype route/helper for `local.workspace.resolve`.
- [x] T4 add small Python helper library for tool/API test driving.
- [x] T4 enforce MCP descriptor status matches the local tool matrix.
- [x] T4 replace generic MCP artifact schemas with request-shaped schemas for implemented artifact/generation tools.
- [x] T4 add representative MCP schema accept/reject conformance tests.
- [x] T5 harden initial component catalogue.
- [x] T5 add explicit failure tests for missing A2UI renderer/schema/topology coverage.
- [x] T5A define rich artifact schemas for slideDeck, slide, spreadsheet, sheet, website, page, form, image, video, audio, chart, diagram, and timeline.
- [x] T5A define Plotly-backed chart API for barChart, lineChart, heatmap, boxPlot, scatterPlot, multi-series, stacked/grouped, horizontal/vertical, fit, and double-axis cases.
- [x] T5A define Mermaid-backed diagram API and structured timeline API.
- [x] T5A define generated media artifact provenance and render path.
- [x] T5A add runtime native artifact validators for generated media, sheet/table, chart, diagram, slide, and slideDeck.
- [x] T5A add native artifact JSON Schema snapshot generated from the Rust contract.
- [x] T5A tighten TS/Python native artifact parsers against the frozen contract shape.
- [x] T5A generate TS/Python native artifact bindings from the schema snapshot with a drift gate.
- [x] T5A add frontend renderer drift coverage for frozen Plotly chart families.
- [x] T5A expand browser E2E chart hydration for bar, heatmap, box plot, and scatter artifacts.
- [x] T5A add Plotly fit-trace and double-axis renderer support with browser assertions.
- [x] T5A promote structured timeline from deferred contract to explicit native artifact/API/renderer.
- [x] T6 add prototype telemetry/audit for workspace records, tasks, and mutations.
- [x] T6 add prototype render-error telemetry intake and server render-miss audit.
- [x] T6 add prototype mutation allowlist and rejection paths.
- [x] T7 run isolated full functional verification.
- [x] T7 add first isolated browser E2E for chat comment/mutate/resolve/reload.
- [x] T7 expand isolated browser E2E to cover server restart persistence and live websocket reconnect.
- [x] T7 expand isolated browser E2E to cover exact sub-element annotation and mutation through Shadow DOM selectors.
- [x] T7 expand isolated browser E2E to cover dark/theme semantic-token rendering across Shadow DOM.
- [x] T7 expand isolated browser E2E to cover render-error UI plus telemetry.
- [x] T7 expand isolated browser E2E to cover richer catalogue flows.
- [x] T7 seed stable artifact sub-element topology ids in renderer annotations.
- [x] T7 add Rust-to-TS artifact topology registry drift gate.
- [x] T7 add browser runtime topology assertions for implemented artifact renderers.
- [x] T7 run isolated telemetry/session inspection.
- [x] T7 run isolated projection/replay/stream/mutation/component benchmarks.
- [x] T7 update changelog and porting docs.
- [ ] T8 lock mutation semantics, renderer/error hardening, persistence/telemetry, and performance refresh before broad catalogue expansion.
- [ ] T8 enforce pure Preline timeline template parity with source/drift tests.
- [ ] T8 define slide/deck compatibility model for PPTX and Google Slides import/export semantics.
- [ ] T8 define spreadsheet/workbook compatibility model for XLS/XLSX and Google Sheets import/export semantics.
- [ ] T8 define JavaScript editing UX contracts for slide, sheet, chart, diagram, timeline, and generated-media artifacts without creating a second document model.
- [x] T8 define `local.export.*` MCP/tool contracts for slideDeck, spreadsheet, chart, diagram, and PDF exports.
- [ ] T8 spike VM-packaged office/PDF export toolchains first: LibreOffice/OpenOffice, Python document libraries, Chromium print, and Pandoc.
- [ ] T8 keep Plotly and Mermaid on the authoritative UI renderer path, with export through the same renderer or a parity-checked headless browser path.
- [ ] T8 use Rust only as export control plane/job runner/audit wall unless a Rust exporter is proven better for a specific artifact.
- [ ] T8 add end-to-end MCP gate: agent dictates a slide deck one slide at a time, edits it through topology, exports it, and receives a downloadable deck file.
- [x] T8 add partial slide-deck export gate: agent creates slides/deck through local tools and receives a downloadable HTML deck file.
- [ ] T8 wire PPTX/PDF slide-deck export to VM-packaged office tooling and promote the deck gate from HTML proof to uploadable PowerPoint-compatible file.
- [x] T8 add end-to-end MCP/local-tool gate: agent creates a spreadsheet with data plus chart, exports it, and receives a downloadable workbook file.
- [ ] T8 extend spreadsheet export gate with topology-edit-before-export once the sheet editor exposes stable editable cell/range mutations.
- [ ] T7 prepare mainline port plan only after lock gates pass.

## Notes

- The prototype proved the interaction shape but still has prototype debt:
  - In-memory workspace.
  - Browser-local comment task list.
  - Numeric Shadow DOM node ids.
  - Prototype-only Rust server.
  - Partial component catalogue.
- Mainline must preserve the discipline: user/model/plugin changes enter as records; UI renders projections.
- Mainline porting is explicitly blocked until the isolated lane is a complete,
  locked, tested, and benchmarked feature. This branch is still prototype and
  design proof work, not a mergeable production feature.
- Priority correction: complete mutation semantics, renderer/error hardening,
  persistence/telemetry lock, and performance refresh before broader editor
  catalogue work or mainline port planning.
- Timeline must be pure Preline component-template code with drift tests, not a
  homegrown visual approximation.
- Slides and spreadsheets are compatibility contracts, not preview widgets:
  deck export must target PowerPoint/PPTX and Google Slides semantics, and
  workbook export must target XLS/XLSX and Google Sheets semantics.
- JavaScript UX is part of the contract: the user must be able to inspect, edit,
  comment, and export the same artifact model agents mutate through MCP/local
  tools.
- Export is part of the proof, not polish. The sprint is not done until MCP can
  drive deck/spreadsheet creation and produce downloadable files.
- The Capsem image should ship office/PDF/rendering tools. Prefer typed
  invocation of LibreOffice/OpenOffice, Python document libraries, Chromium
  print, Pandoc, or similar tooling over rebuilding document exporters in Rust
  or JavaScript.
- Plotly and Mermaid are different: they must render in the UI, so chart/diagram
  export should reuse the authoritative UI renderer or a parity-checked headless
  browser path rather than a separate VM-assist renderer.
- Rust is the export control plane and audit wall, not the default place to
  implement PPTX/XLSX/PDF internals.
- Added the first export control-plane slice: `local.export.spreadsheet` and
  `local.export.slideDeck` descriptors/routes, Python client helpers, export
  telemetry, static file serving for generated exports, and
  `scripts/export_artifact.py` as the VM-tool adapter.
- Spreadsheet XLSX export is real through bundled Python `openpyxl`, including
  basic chart embedding when a chart artifact references the sheet. Slide-deck
  PPTX/PDF export now returns a typed `toolUnavailable` result until
  python-pptx, LibreOffice/OpenOffice, or another office toolchain is present.
- Added `npm run inspect:export`: starts the Rust prototype server, creates a
  sheet/chart and slides/deck through the local tool routes, calls
  `local.export.spreadsheet` and `local.export.slideDeck`, downloads the
  exported XLSX and HTML deck, inspects the XLSX with `openpyxl` for
  header/data rows plus an embedded chart sheet, verifies deck HTML content,
  and asserts PPTX remains a typed `toolUnavailable` result.
- The production UI work is large enough to be its own delivery stream, not a side effect of plugin/WASM integration.
- The first production proof should be boring and strict: one card, one comment, one mutation, one resolve, one restart, one telemetry inspection.
- A2UI Basic has more primitives than the current `ui.*` helper layer. Current helpers are Pack 01, not full A2UI coverage.
- Protocol alternatives must be tested before implementation. CRDTs are only one family of candidates.
- Yrs/Yjs and Automerge were compared against Capsem's authoritative append log and the A2UI envelope option; Loro is the only selected CRDT dependency path.
- The local MCP/tool surface is part of the product contract and must be typed/tested before integration.
- Rich artifacts are part of the runtime contract from day one: slide decks, slides, spreadsheets, sheets, websites/forms, generated image/video/audio, Plotly charts, Mermaid diagrams, and timelines.
- Protocol spike selected Capsem record envelope plus Loro UI/artifact state. Automerge and Yrs/Yjs were considered and rejected for this sprint; do not add them as parallel backends.
- Loro audit fixture passed in Rust and browser for card, slide deck, sheet, and website/form edits. Fixture size: 2,181-byte base snapshot, 196-byte update, 2,304-byte updated snapshot.
- Loro adds a non-trivial Rust dependency tree. This is accepted for the spike because it remains the only CRDT backend, but production integration should keep the dependency audit visible.
- Added `contract_matrix` as the machine-readable boundary source for A2UI Basic primitives, Pack 01 helpers, promoted renderer templates, Capsem extension artifacts, and local MCP tools.
- Added Pack 01 matrix-driven conformance test proving every shipped helper lowers to A2UI wire messages, declares a matching renderer recipe, and emits stable caller-id-derived component ids.
- Added `capsem-ui-runtime` as the reusable Rust dependency seam for workspace projection/checkpoints and Loro editable state. Prototype server workspace routes now import reducer types through this crate.
- T1 extraction is currently a seam/re-export extraction. Full ownership move remains blocked on splitting artifact construction/types out of `capsem-ui-catalog`.
- Added first-class `WorkspaceTask` projection state plus `upsertTask` deltas. `ui.change` request records now create durable open tasks; `ui.resolve` response records close them; checkpoints carry task state.
- Browser native workspace parsing now validates `tasks` and `upsertTask`, including annotation selector metadata, so live stream and cold replay share the same contract.
- Reducer now rejects malformed known task actions (`ui.change`, `ui.resolve`) and change requests for missing elements instead of silently ignoring broken records.
- Added `capsem-ui-runtime::storage::SqliteWorkspaceStore` with tables for workspace records and checkpoints. Runtime tests prove no-checkpoint replay and checkpoint-plus-tail replay after reopening the database.
- Prototype server can now restore `NativeWorkspace` from `CAPSEM_NATIVE_WORKSPACE_DB`, persist workspace mutation records before broadcasting, clear persisted state on reset, and save checkpoints through `POST /native/workspace/checkpoint`.
- Browser native workspace helper now includes `checkpointWorkspace()` so the checkpoint route is part of the test-driving surface.
- Added `POST /native/workspace/resolve`, `local__workspace_resolve`, and `resolveWorkspaceTask()` so task resolution travels through the same record/projection/persist/broadcast lane as comments and mutations.
- Extended `python/capsem_native_client` with a typed `workspace` helper namespace covering reset, info, snapshot, projection, checkpoint, select, comment, resolve, title, style, and delete.
- Reworked `/native/mcp/tools` to expose canonical dotted tool names with adapter names, methods, routes, descriptions, and JSON-schema-shaped input contracts. Added unified `POST /native/workspace/mutate` plus Python/TS helpers for `local.ui.mutate` title/style mutations.
- `/native/mcp/tools` now includes every frozen `LOCAL_TOOL_COVERAGE` matrix tool exactly once, marks unsupported future tools as `status: "deferred"`, and allows prototype-only convenience descriptors separately.
- Added first-class text records and text patch mutations to the workspace reducer. `local.ui.mutate` now advertises and has TypeScript/Python helpers for title, text, and style mutation variants.
- Hardened `SqliteWorkspaceStore` so workspace records are strict append-only sequence entries, duplicate/gap records are rejected, and checkpoints cannot be saved ahead of persisted records or behind the latest checkpoint.
- Added prototype workspace telemetry events alongside artifact telemetry. Mutations now audit seq, record id, role, principal, verb, content type, status, target, task id, delta types, compact mutation summary, and timing before the later mainline/session telemetry port.
- Added a prototype style-mutation allowlist at the Rust reducer wall, aligned the `local.ui.mutate` descriptor with the allowed properties, and added rejection tests for unsupported properties and dangerous values.
- Added `local.ui.renderError`/`POST /native/ui/render-error` and wired the chat shell to report `capsem:error` events from Web Components. Missing artifact render requests now also emit `renderError` telemetry before returning 404.
- Added a Playwright-backed `npm run e2e:chat` gate. The test starts the Rust prototype server with a temporary SQLite workspace, creates a table artifact, opens `/chat`, submits a title comment, mutates the title through Rust, restarts the server against the same SQLite DB, proves the open browser reconnects over websocket, reloads the page, resolves the durable task through Rust, and asserts workspace telemetry on both sides of the restart.
- Fixed the chat shell to hydrate comment tasks from Rust projection state after reload and to resolve tasks through `POST /native/workspace/resolve` instead of local-only state changes.
- Expanded the browser E2E to click a specific table row inside a Web Component Shadow DOM, persist its composed `hostSelector >>> shadowSelector`, verify captured row metadata, apply a Rust-allowlisted style mutation to only that row, and replay the edit after server restart plus browser reload.
- Fixed the `local.ui.mutate` style request boundary so camelCase `hostSelector`, `shadowSelector`, and `sourceRequestSeq` deserialize into Rust enum payloads; added a server contract test using the exact MCP-shaped JSON.
- Expanded the browser E2E to switch Preline theme plus dark mode, verify document/localStorage state, and assert Shadow DOM artifact text inherits the host's resolved semantic foreground token before and after reload.
- Added a visible chat render-error panel fed by Web Component `capsem:error` events. Browser E2E now seeds an invalid Mermaid diagram, proves the render error appears in the UI, and polls telemetry for the matching `render.error` audit event.
- Web Component chart/diagram renderers now include the underlying renderer exception message in `capsem:error` payloads so UI and telemetry report the specific failure, not only the renderer name.
- Expanded browser E2E catalogue coverage beyond the table flow: the same run now creates a Plotly bar chart and a valid Mermaid diagram, then asserts their Shadow DOM renderer output is hydrated.
- Added `npm run bench:workspace`, an isolated workspace runtime benchmark that starts the Rust prototype server with a temporary SQLite workspace, seeds 24 tables plus chart/diagram artifacts, measures projection, snapshot, mutation, checkpoint, websocket fanout, restore, and browser render timings, and prints a single normalized table.
- Recorded the 2026-06-06 workspace runtime benchmark in `docs/workspace-runtime-benchmark.md`: projection p95 `1.173ms`, snapshot p95 `1.881ms`, title mutation p95 `0.694ms`, checkpoint p95 `1.914ms`, 5-client stream fanout p95 `1.999ms`, restore+server-start p95 `1545.6ms`, browser render p95 `613.4ms`.
- Added `npm run inspect:workspace`, an isolated session inspection gate that drives create/comment/mutate/render-error/resolve/checkpoint through the Rust prototype API, asserts live telemetry contracts, stops the server, and inspects the SQLite record/checkpoint store directly.
- Recorded the 2026-06-06 workspace session inspection in `docs/workspace-session-inspection.md`: 7 live telemetry events, 5 durable records through seq 5, one checkpoint at seq 5, and coverage for task/title/style/resolve records.
- Updated `sprints/mainline-ui-runtime-integration/porting.md` with mainline owner inventory, selected Capsem-record-plus-Loro guidance, local tool contracts, telemetry contract fields, and mainline test-gate equivalents.
- Full isolated gate passed on 2026-06-06 with Rust contract tests, TypeScript typecheck, Vitest, Python client tests, browser E2E, session inspection, workspace benchmark, and `git diff --check`.
- Hardened `CAPSEM_BLOCK_COVERAGE` so every Pack 01 block declares schema, trusted renderer adapter, semantic topology targets, concrete emitted A2UI component ids, and mutations. The contract matrix now fails if a helper stops emitting the ids its renderer/topology contract promises.
- Upgraded the explicit Svelte renderer test to consume `CAPSEM_BLOCK_COVERAGE`, so matrix-declared renderer adapters must correspond to real `A2Node.svelte` recipe branches.
- Added `RICH_ARTIFACT_SCHEMA_COVERAGE` for day-one slideDeck, slide, spreadsheet, sheet, website, page, form, image, video, audio, chart, diagram, and timeline families. The matrix freezes schema names, required fields, optional fields, renderer backend, topology targets, and mutation surface.
- Added `PLOTLY_CHART_API_COVERAGE` for `ui.chart.barChart`, `lineChart`, `heatmap`, `boxPlot`, and `scatterPlot`, including required fields, Plotly trace type, export formats, and support flags for direction, stack, fit, and second-axis cases.
- Expanded `ChartRequest` from bool `stack` to enum stack mode, added `ScatterPlot`, optional `xUnit`, optional fit metadata, and Python client coverage for the new chart payload shape.
- Added `MERMAID_DIAGRAM_API_COVERAGE` and `TIMELINE_API_COVERAGE`; Mermaid remains strict-sandbox source rendering, while timeline is a structured event/lane artifact. Added deferred `local.ui.timeline` descriptor coverage so the tool name is reserved without becoming callable.
- Added `GENERATED_MEDIA_API_COVERAGE` for text, image, embedding, video, and audio generation. Text/image/embedding are explicit; video/audio remain deferred, but all rows freeze required fields, provenance, renderer, usage/cost telemetry, and status expectations.
- Added `validate_native_artifact()` as the first runtime validator wall for emitted native artifacts. Constructors now refuse malformed generated media, sheet/table, chart, diagram, slide, and slideDeck specs; tests cover demo artifact conformance, generated-media telemetry placeholders, unsupported Plotly kinds, and incompatible chart options.
- Added `native_artifact_schema()` plus the checked-in `schemas/capsem-ui/artifacts/native-artifact.v1.schema.json` snapshot. A Rust test proves the snapshot and Rust contract remain isomorphic; generated TS/Python bindings from the schema remain pending.
- Tightened `ui-preview/src/nativeDeck.ts` from loose `Record<string, unknown>` artifact specs to discriminated native artifact/spec types and parser checks. Tightened the Python helper with `parse_native_artifact()` so `native.artifact(s)` rejects malformed native artifact responses before agents consume them.
- Added `scripts/generate-native-artifact-types.ts`, `npm run generate:artifact-types`, and `npm run check:artifact-types`. Generated outputs now live at `ui-preview/src/generated/nativeArtifact.ts` and `python/capsem_native_client/generated_native_artifact.py`; TS/Python parsers consume those bindings/constants instead of maintaining parallel enum/component lists.
- Added frontend source-level renderer drift coverage for the Plotly chart catalogue so line/scatter, heatmap, box plot, horizontal bar, stack mode, and export branches are all named in tests. This is not a substitute for browser visual tests.
- Expanded `npm run e2e:chat` so the browser path now creates Plotly bar, heatmap, box plot, and scatter artifacts through Rust, waits for each Shadow DOM Plotly renderer to hydrate, and keeps the existing restart/comment/mutation flow green.
- Added real Plotly renderer support for `fit: { method: "linear" }` on line/scatter charts and `secondAxis`/right-axis series via `y2`. Browser E2E now asserts the generated linear-fit trace and the right-axis Plotly layout/trace state.
- Promoted timeline from reserved contract to callable prototype artifact: `TimelineRequest`/lane/event structs, `POST /native/ui/timeline`, explicit `local.ui.timeline`, Rust schema snapshot coverage, generated TS/Python bindings/constants, Python `ui.timeline()`, `capsem-timeline`, source renderer drift coverage, and browser E2E hydration via the Rust server.
- Added a server contract gate that compares every `/native/mcp/tools` descriptor status against `LOCAL_TOOL_COVERAGE`. Website render/form descriptors are now deferred in the matrix as well as the server until they have real routes and schemas.
- Tightened implemented MCP input schemas for generated text/image/embedding, sheet/spreadsheet, table, chart, Mermaid diagram, timeline, slide, and slide deck. The server test now freezes chart enums/options, timeline lane/event requirements, slide block unions, and slide deck refs.
- Added a test-only MCP schema evaluator covering the emitted schema subset. Representative chart, timeline, slide, and slideDeck payloads now prove valid requests pass and malformed enum/missing-field/block/ref payloads fail.
- Added stable `data-capsem-topology-id`/`data-capsem-topology-role` stamping across artifact renderers. Annotation payloads and durable `WorkspaceAnnotationTarget` now preserve `topologyId`/`topologyRole`; chat marker grouping prefers topology id, while composed selectors remain as fallback/debug. Browser E2E now requires the Lannister row selector to be `e2e-house-table::row::1`.
- Added `schemas/capsem-ui/artifacts/topology-targets.v1.json` as the Rust-generated rich artifact topology snapshot and `ARTIFACT_TOPOLOGY_REGISTRY` as the TypeScript renderer-side declaration. Implemented renderer families must match Rust-declared roles; deferred families must stay explicitly empty.
- Updated artifact renderer stamping to use SVG-safe `setAttribute` topology metadata so Mermaid nodes/edges can participate in the same annotation identity model as HTML elements.
- Expanded `npm run e2e:chat` to create sheet, chart, Mermaid diagram, generated image, timeline, slide, and slide deck artifacts through Rust and assert their declared topology roles in the live Shadow DOM. The E2E now also proves successful Mermaid hydration preserves a hidden `source` topology node after replacing the fallback code block with SVG.
- Added optional generated-image `caption` to the Rust request/spec contract, native artifact schema snapshot, and generated TS/Python bindings; the media renderer now emits a `controls` topology node so generated images cover the full declared `title/media/caption/controls/source` topology family.
- Expanded exact-element feedback E2E beyond table rows: generated-image caption and slide text-block comments now assert stable `data-capsem-node` selectors, topology ids, topology roles, task projection, restart, and reload persistence.
- Added selector-scoped `textPatches` beside `stylePatches` so `local.ui.mutate` can edit artifact sub-elements by stable topology selector. The path is covered in Rust reducer tests, server descriptor/deserializer tests, TS/Python helper tests, Shadow DOM renderer replay, telemetry mutation summaries, and browser E2E for generated-image captions and slide text blocks across restart/reload.
- Hardened selector-scoped text patches to require stable `data-capsem-node` or `data-capsem-topology-id` selectors at the Rust reducer wall and in the MCP descriptor schema. Arbitrary selectors such as `[part~="body"]` are rejected by reducer and descriptor tests.

## Coverage Ledger

- Unit/contract: runtime crate boundary tests, Loro audit tests, Pack 01 conformance tests, workspace reducer tests, runtime SQLite storage/adversarial tests, server restore/resolve/descriptor/telemetry/render-error tests, native workspace TS parser/API tests, and Python client route tests pass; comment/create/resolve/checkpoint lifecycle, title/text/style mutations, malformed task-action rejection, restart replay, append-only storage, descriptor schemas, workspace telemetry schema, render-error telemetry schema, mutation allowlist, and helper call shapes are covered.
- Functional: prototype route wiring exists behind `CAPSEM_NATIVE_WORKSPACE_DB`; full isolated verification now passes for table/chart/diagram creation, comment/title-mutate/exact Shadow DOM row/image-caption/slide-text comment/style-mutate/text-mutate/theme-dark token rendering/render-error UI/server-restart/websocket-reconnect/reload/resolve/telemetry/session inspection/export inspection/benchmark; mainline routes/tools are blocked until lock gates pass.
- A2UI conformance: matrix coverage plus Pack 01 helper-to-wire-to-render-id tests added; Pack 01 now checks schema, exact renderer recipe, trusted renderer adapter, semantic topology targets, concrete emitted component ids, and mutation declarations.
- Artifact conformance: taxonomy matrix, rich artifact schema family matrix, Plotly chart API matrix, Mermaid/timeline API matrices, generated-media provenance matrix, first runtime native artifact validators, checked Rust-generated native artifact JSON Schema snapshot, checked Rust-generated topology target snapshot, generated TS/Python native artifact bindings, stricter TS/Python native artifact parsers, source-level Plotly/timeline/topology renderer drift coverage, browser hydration coverage for bar/heatmap/box/scatter/timeline artifacts, browser runtime topology assertions for sheet/chart/diagram/image/timeline/slide/slideDeck, and browser assertions for fit/double-axis Plotly state are added; validators for not-yet-constructed families, exhaustive editor topology tests, and broader browser visual tests are still pending.
- Protocol bake-off: selected-protocol and logical fixture complete; Loro API audit fixture passed in Rust and browser.
- MCP/tool contract: `local.*` hierarchy frozen in matrix, server descriptors must match matrix status with implemented tools declaring adapter/method/route and deferred tools declaring none, implemented generation/artifact/export tools now expose request-shaped schemas with enum/union coverage, and representative schema accept/reject tests pass; exhaustive generated schema validation and mainline MCP wiring remain pending.
- Adversarial: reducer malformed action/missing target tests, selector-scoped text mutation topology-selector rejection tests, style mutation allowlist rejection tests, and storage duplicate/gap/stale checkpoint tests pass; browser/mainline adversarial E2E still pending.
- E2E/browser: `npm run e2e:chat` passes for sheet/table/chart/diagram/generated-image/timeline/slide/slideDeck creation, Plotly, Mermaid, and timeline Shadow DOM hydration, declared topology-role assertions for implemented runtime fixtures, comment/title-mutate/exact stable row/image-caption/slide-text topology annotations/style-mutate/text-mutate/theme-dark token rendering/render-error UI/restart/reconnect/reload/resolve; `npm run inspect:export` passes for local-tool sheet/chart/slide/deck creation, XLSX export, HTML deck export, file downloads, workbook row inspection, chart-sheet inspection, typed PPTX unavailable handling, and export telemetry; deeper catalogue/editor flows remain pending before mainline porting.
- Telemetry: prototype workspace record telemetry covers principal/target/verb/task/mutation/timing before and after restart in the E2E; export telemetry now records artifact id, format, adapter, status, output path, bytes, message, and timing; `npm run inspect:workspace` now proves live telemetry plus durable SQLite record/checkpoint inspection for create/comment/mutate/render-error/resolve/checkpoint; prototype telemetry remains in-memory across process restarts; prototype render-error telemetry has both unit coverage and browser E2E coverage for a real Mermaid renderer failure, plus server render-miss coverage; full mainline session DB inspection after a real run remains pending.
- Performance: `npm run bench:workspace` covers projection, snapshot/replay shape, checkpoint, stream fanout, mutation latency, restore+server-start, and browser component rendering. 2026-06-06 default run is recorded in `docs/workspace-runtime-benchmark.md`; pure mainline service restore benchmarks remain pending because this prototype still includes process startup.
- Missing/deferred: WASM/plugin execution, slide-deck PPTX/PDF office toolchain, chart/diagram export through the authoritative browser renderer path, polished artifact editors/export paths, exhaustive artifact renderer/topology tests beyond the seeded stable ids, telemetry persistence/session DB wiring, mainline session inspection, mainline service lifecycle wiring for the SQLite store, and the final move of workspace implementation ownership into `capsem-ui-runtime`.

## Release Hold

This sprint is not releasable until:

- Mainline porting remains blocked until the prototype is fully functional,
  locked, tested, benchmarked, and reviewed.
- Comments are durable prototype records/tasks, not local UI state; mainline persistence and port still pending.
- Runtime and prototype service restart persistence pass for the first chat loop; broader restart cases and mainline restart wiring still pending.
- Mainline browser E2E passes.
- `local.ui.info` and mutation tools operate through the same Rust projection engine.
- Component topology ids are stable for all shipped components.
- Telemetry/audit captures principal, target, mutation, result, and timing.
- T0A protocol bake-off is complete and recorded before T1 implementation starts.
- A2UI missing primitives are either implemented or explicitly deferred by pack.
- MCP/local tool surface is frozen for the first PR.
- Rich artifact taxonomy is frozen enough to support slide/spreadsheet/website/form/media/chart/diagram/timeline work without changing the runtime spine.
