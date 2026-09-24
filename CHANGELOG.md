# Changelog

## [Unreleased]

### Added

- Added prototype `local.export.spreadsheet` and `local.export.slideDeck`
  control-plane routes with MCP descriptors, export telemetry, Python client
  helpers, and a VM-tool adapter that writes real XLSX workbooks through
  `openpyxl` while reporting missing slide-deck office tooling explicitly.
- Added `npm run inspect:export`, an end-to-end export gate that starts the
  Rust prototype server, creates a sheet plus chart through the local tool
  routes, exports an XLSX file, downloads it, inspects workbook rows plus
  chart-sheet presence with `openpyxl`, exports a downloadable slide-deck HTML
  file, and verifies PPTX remains a typed unavailable result until office
  tooling is configured.
- Added a Loro-backed editable UI/artifact state audit fixture in Rust and
  browser tests for cards, slide decks, sheets, website forms, stable topology
  ids, import/export, and Capsem record wrapping.
- Added a machine-readable UI contract matrix for A2UI Basic coverage, Pack 01
  helpers, Capsem extension artifacts, and the frozen `local.*` tool hierarchy.
- Added the `capsem-ui-runtime` crate boundary for workspace projection,
  checkpoints, and Loro editable state so service/MCP code can depend on a
  runtime seam instead of the UI catalogue directly.
- Changed the prototype server workspace route code to import projection,
  topology, checkpoint, and mutation record types through `capsem-ui-runtime`.
- Added first-class workspace task projection state: `ui.change` request records
  now create durable open tasks, `ui.resolve` response records close them,
  checkpoints carry task state, and the browser stream parser validates
  `upsertTask` deltas.
- Hardened workspace task projection so malformed `ui.change`/`ui.resolve`
  payloads and change requests for missing elements fail loudly in reducer
  tests instead of disappearing during replay.
- Added a runtime SQLite workspace store for durable records and checkpoints,
  with restart tests proving no-checkpoint replay and checkpoint-plus-tail
  replay restore elements, tasks, projection state, and snapshots.
- Wired the prototype native workspace service to optional
  `CAPSEM_NATIVE_WORKSPACE_DB` persistence, including boot restore,
  persist-before-broadcast mutation handling, reset clearing, checkpoint saving,
  and a browser helper for `POST /native/workspace/checkpoint`.
- Added prototype task-resolution plumbing with `POST /native/workspace/resolve`,
  `local__workspace_resolve`, and a browser `resolveWorkspaceTask()` helper so
  resolved comments update through the same workspace record lane.
- Extended the Python native client with a `workspace` namespace for
  reset/info/snapshot/projection/checkpoint/select/comment/resolve/title/style/delete
  calls, giving agents one typed rehearsal surface for the local UI tools.
- Reworked the prototype MCP tool descriptor endpoint to publish canonical
  dotted tool names, adapter names, methods, routes, descriptions, and
  JSON-schema-shaped inputs, including a unified `local.ui.mutate` route with
  TypeScript and Python helpers for title/style mutations.
- Added matrix coverage checks for the prototype MCP descriptor endpoint so
  every frozen `LOCAL_TOOL_COVERAGE` tool appears once and unsupported future
  tools are advertised as deferred instead of hidden or half-callable.
- Added first-class workspace text records and text patch mutations, including
  reducer tests and `local.ui.mutate` TypeScript/Python helpers for title,
  text, and style mutation variants.
- Hardened runtime workspace persistence so records are strict append-only
  sequence entries and checkpoints cannot be saved ahead of persisted records
  or behind the latest checkpoint.
- Added prototype workspace telemetry events for records, tasks, and mutations,
  including principal, target, verb, task id, delta types, mutation summary,
  status, and timing alongside existing artifact construction telemetry.
- Added a Rust reducer allowlist for style mutations, with descriptor coverage
  for allowed `local.ui.mutate` style keys and adversarial tests rejecting
  unsupported properties or unsafe CSS values.
- Added prototype render-error telemetry with `local.ui.renderError`,
  `/native/ui/render-error`, chat-shell `capsem:error` reporting, and
  server-side render-miss audit events.
- Added a visible chat render-error panel for Web Component renderer failures,
  and expanded `npm run e2e:chat` to prove an invalid Mermaid diagram appears
  in the UI and emits matching `render.error` telemetry.
- Expanded `npm run e2e:chat` catalogue coverage to create a Plotly bar chart
  and valid Mermaid diagram, then assert both renderers hydrate inside Shadow
  DOM.
- Added `npm run bench:workspace`, an isolated workspace runtime benchmark for
  projection, snapshot/replay shape, mutation, checkpoint, websocket fanout,
  restore, and browser renderer hydration.
- Documented the first workspace runtime benchmark in
  `docs/workspace-runtime-benchmark.md`.
- Added `npm run inspect:workspace`, an isolated session inspection gate that
  drives create/comment/mutate/render-error/resolve/checkpoint over the Rust
  prototype API, then verifies live telemetry and durable SQLite records.
- Documented the first workspace session inspection in
  `docs/workspace-session-inspection.md`.
- Expanded the mainline UI runtime porting map with owner inventory, selected
  protocol guidance, tool/telemetry contracts, and the test gates required
  before integration.
- Hardened the Pack 01 UI catalogue matrix so shipped helpers declare their
  A2UI schema, trusted Svelte renderer adapter, semantic topology targets,
  concrete emitted component ids, and mutation surface.
- Expanded catalogue drift tests so Rust helpers must emit the component ids
  declared by the matrix and every matrix renderer adapter must have an
  explicit `A2Node.svelte` branch.
- Added a rich artifact schema coverage matrix for day-one slide deck, slide,
  spreadsheet, sheet, website, page, form, image, video, audio, chart, diagram,
  and timeline families, with required fields, renderer, topology, and mutation
  declarations.
- Added a Plotly chart API coverage matrix for bar, line, heatmap, box, and
  scatter chart families, and expanded the native chart request shape with
  scatter plots, enum stack modes, optional x units, fit metadata, and
  double-axis/export fields.
- Added Mermaid diagram and structured timeline API coverage matrices, and
  advertised `local.ui.timeline` as a deferred local tool so timeline work has
  a named contract without becoming half-callable.
- Promoted structured timeline artifacts into the prototype runtime with
  `POST /native/ui/timeline`, an explicit `local.ui.timeline` descriptor,
  Rust validation/schema coverage, generated TS/Python bindings, a
  `capsem-timeline` renderer, and browser E2E hydration coverage.
- Hardened `/native/mcp/tools` contract tests so every descriptor status must
  match `LOCAL_TOOL_COVERAGE`; website render/form tools are now honestly
  deferred until callable routes exist.
- Replaced generic MCP input schemas for generated media, sheet/table, chart,
  diagram, timeline, slide, and slide deck tools with request-shaped JSON
  schemas that expose required fields, enums, block unions, and export formats.
- Added a test-only MCP JSON Schema conformance gate that validates
  representative chart, timeline, slide, and slide deck payloads are accepted
  while malformed enum, missing-field, and block/ref payloads are rejected.
- Added stable artifact sub-element topology ids/roles to the Web Component
  renderers and persisted annotation contract so comments can target rows,
  cells, timeline events, slide blocks, chart parts, and media nodes without
  relying only on fallback DOM traversal selectors.
- Added a checked artifact topology snapshot plus a TypeScript renderer
  topology registry so Rust-declared rich artifact topology targets and
  implemented Shadow DOM renderer roles drift together under test.
- Expanded the chat browser E2E with runtime topology assertions for sheet,
  chart, Mermaid diagram, generated image, timeline, slide, and slide deck
  artifacts, including hydrated SVG node/edge stamping and post-render source
  preservation.
- Added optional generated-image captions to the Rust native artifact contract,
  schema snapshot, generated TypeScript/Python bindings, and renderer topology
  path so image artifacts can expose caption and controls roles under test.
- Expanded the chat browser E2E exact-element feedback path beyond table rows
  to generated-image captions and slide text blocks, proving richer topology
  targets persist as workspace tasks across restart and reload.
- Added selector-scoped text mutations for artifact sub-elements, including
  Rust projection storage, MCP descriptor schema, TypeScript/Python helpers,
  Shadow DOM renderer replay, telemetry counts, and browser E2E coverage for
  generated-image captions and slide text blocks.
- Hardened selector-scoped text mutations so Rust and the MCP descriptor only
  accept stable Capsem topology selectors, rejecting arbitrary DOM selector
  edits before they reach renderer replay.
- Added generated media API coverage for text, image, embedding, video, and
  audio, including provenance, renderer, telemetry usage/cost, and deferred
  status for video/audio generation.
- Added runtime native artifact validation against the frozen catalogue
  matrices, including constructor enforcement for generated media telemetry
  shape, table/sheet/deck required fields, supported Plotly chart kinds, chart
  option compatibility, Mermaid diagram shape, and slide/deck block structure.
- Added a Rust-generated native artifact JSON Schema snapshot at
  `schemas/capsem-ui/artifacts/native-artifact.v1.schema.json`, with a test
  proving the checked-in schema stays isomorphic with the Rust contract.
- Tightened TypeScript and Python native artifact clients with discriminated
  artifact/spec contracts plus parser tests that reject malformed chart specs
  before the UI or agent helper code renders them.
- Added schema-driven native artifact binding generation for TypeScript and
  Python, including `npm run generate:artifact-types` and
  `npm run check:artifact-types` drift gates.
- Added frontend renderer drift coverage for the frozen Plotly chart catalogue,
  including line/scatter, heatmap, box plot, horizontal bar, stack mode, and
  SVG/PNG export branches.
- Expanded the chat browser E2E to create and hydrate Plotly bar, heatmap, box
  plot, and scatter chart artifacts through the Rust API and Shadow DOM
  renderer.
- Added Plotly renderer support and browser assertions for linear fit traces
  and second-y-axis charts.
- Added `npm run e2e:chat`, a Playwright-backed isolated browser gate that
  starts the Rust prototype server, creates an artifact, comments on the chat
  UI, mutates through Rust, reloads, resolves the durable task, and checks
  workspace telemetry.
- Expanded `npm run e2e:chat` to restart the Rust prototype server against the
  same SQLite workspace, prove the open browser reconnects over websocket, and
  verify the durable task and latest title survive reload after restart.
- Expanded `npm run e2e:chat` to cover exact Shadow DOM sub-element editing:
  the test comments on a specific table row, persists the composed selector,
  applies an allowlisted style mutation to that row, and verifies replay after
  restart and reload.
- Expanded `npm run e2e:chat` to verify Preline theme plus dark-mode state and
  Shadow DOM semantic-token color inheritance before and after reload.
- Changed chart and diagram Web Component error payloads to include the
  underlying renderer exception message for UI display and telemetry.
- Fixed `local.ui.mutate` style request deserialization so camelCase
  `hostSelector`, `shadowSelector`, and `sourceRequestSeq` survive the Rust
  enum boundary.
- Added a reusable `<capsem-elt>` Web Component foundation with Shadow DOM,
  typed spec assignment, lifecycle cleanup, and composed Capsem event emission.
- Documented the future spreadsheet-to-slide-deck UI track so chart, diagram,
  and export work preserves sheet ranges, export handles, slide blocks, and
  deck composition.
- Documented the future per-instance SQLite workspace requirement for
  model-assisted data manipulation before rendering sheets, charts, decks, or
  other data-backed artifacts.
- Clarified that the SQLite data workbench should first surface through
  Capsem's local MCP tools so agents can use it before the WASM plugin runtime.
- Documented future `web.preview` and `generate.image/video/audio` tracks for
  user-visible previews and typed generated media assets.
- Documented the Gemini-first `generate.*` spike, multimodal media inputs, and
  Capsem MCP tool grouping requirements.
- Aligned future Capsem MCP tool naming notes with the real guest MCP
  `local__snake_case_tool` convention instead of dotted pseudo-tool names.
- Added the native MCP/deck proof sprint covering tool hierarchy, Gemini-backed
  generation, Web Component and Plotly security constraints, and a Python client
  rehearsal layer before MCP wiring.
- Clarified that slide-deck preview uses Capsem-owned `<capsem-*>` components,
  while `web.preview` is reserved for website/page preview, and required
  individual artifact materialization during deck construction.
- Added the first native deck-proof Rust/Python path with read-only SQLite
  query support, individual artifact materialization, multiple chart specs, and
  deterministic artifact handles.
- Added the "Realms Of Code" native deck preview: SQLite-backed house data,
  overview table, two chart specs, diagram specs, planned generated house
  images, one slide per house, and a `/deck` page rendered through
  `<capsem-*>` custom elements.
- Added browser-backed coverage for native artifact components, including a fix
  for custom-element registration so each tag uses its own subclass on the
  shared Capsem artifact foundation.
- Added first-class native artifact construction routes and Python client
  methods for generation, sheet, table, chart, diagram, slide, and slideDeck
  assembly so decks are built by orchestrating typed artifacts instead of a
  hidden deck engine.
- Added a live native artifact workspace with reset/list/render behavior so
  step-by-step tool calls can build an interactive slide deck visible on
  `/deck` without changing code.
- Added the first Gemini-backed image generation path with typed missing-key
  and provider-error artifact states, plus live native telemetry records for
  artifact construction calls.
- Added Capsem AI usage/cost accounting fields for generated artifacts and
  telemetry, plus an embedding generation route for future dashboard and
  retrieval work.
- Added ignored live smoke tests for OpenAI image generation, Gemini Nano Banana
  image generation, and OpenAI embeddings so provider credentials can be checked
  without making default test runs spend money.
- Added Plotly and Mermaid preview/export adapters for native chart and diagram
  artifacts, and an MCP-shaped native tool descriptor endpoint for the eventual
  local tool wrapper.
- Added a reusable Rust `capsem-ai` lane backed by Siumai for provider-owned
  model calls, Capsem-compatible service credential resolution, text generation,
  and image generation artifact routing without custom Gemini HTTP glue.
- Added the isolated UI tool workbench lane: pinned A2UI schemas, Capsem UI
  catalog/template schemas, checked Preline templates, Rust UI tool runner
  endpoints, and a Svelte workbench that renders the tool-built acceptance
  surface first.
- Added a live UI authoring lane so `POST /ui/tools/run` stores the latest
  structured tool result, `/ui/spec/demo` exposes it, and the workbench renders
  the authored "Agent Draft" first.
- Added a chat-shell workbench view that renders the authored A2UI component
  inside an assistant message while preserving the inspector panes.
- Added a clean `/chat` page for the user-facing chat shell, separate from the
  diagnostic workbench.
- Added a Hyper-only A2UI contract lane with E2E conformance tests and a tested
  TypeScript response decoder.
- Added contract support for alert tone metadata and card link metadata so
  accepted `ui.alert` and `ui.card` fields survive to rendering.
- Added `ui.table` as a typed Rust UI tool with A2UI conformance coverage and
  an explicit Preline table renderer.
- Added a renderer-drift contract test so typed Rust UI recipe components fail
  if the Svelte renderer has no explicit adapter.
- Added `/chat` theme and dark-mode controls backed by real Preline packaged
  themes for semantic-token visual checks.
- Added a semantic-token regression test for the generated UI renderer path.
- Added a standalone `capsem-ui-catalog` crate so the A2UI/Preline catalogue can
  land independently of the full WASM plugin runtime.
- Added UI Catalogue Pack 01: `ui.notice`, image/action cards, `ui.facts`,
  searchable/filterable/paginated tables, and choice-based `ui.ask`.
- Added a Rust-owned A2UI v0.9 Basic preview path with typed UI helpers,
  contract tests, server endpoints, and a Svelte/Preline renderer preview.
- Added an A2UI/Capsem/Preline template-system sprint and design note covering
  JSON Schema source of truth, Python Preline scraping, checked template
  bindings, and a Svelte workbench.
- Added the UI MCP-style authoring acceptance gate: the agent must build a
  requested UI through structured tools and render it in the workbench.

### Changed

- Changed the chat shell to hydrate comment tasks from Rust projection state
  after reload and to resolve tasks through `POST /native/workspace/resolve`
  instead of local-only UI state.
- Changed `ui.ask` from a modal launcher to an inline decision card with a
  title, optional detail text, and two visible action buttons.
- Updated the A2UI modal renderer with Svelte dialog behavior: focus restore,
  Escape/backdrop close, scroll lock, and keyboard focus cycling.
- Removed the internal `A2UI Basic component: Card` caption from user-facing
  card output so protocol labels do not bleed into generated UI.
- Fixed generated button labels so nested A2UI text inherits the button
  foreground color instead of rendering muted grey on primary buttons.
- Replaced remaining raw palette utilities in the generated UI renderer path
  with Preline semantic token classes.
- Moved A2UI message types, typed UI helpers, UI tools, and template checks out
  of `capsem-plugin-engine` ownership and into `capsem-ui-catalog`.
- Updated `/chat` to render every authored UI surface in a tool result so block
  packs can be previewed together.
- Updated the UI preview to show the actual A2UI Basic `Card` component with
  Preline card tokens, and cross-check Rust messages with `a2ui-types`.
- Moved the rendered preview above the A2UI JSON and restyled the shell around
  Preline's docs/card theme.
- Replaced the preview's Tailwind CDN/token mimic with the real `preline`
  package processed through the Tailwind v4 Vite pipeline.
- Remapped the UI preview renderer from generic A2UI tree dumping to
  Preline recipe adapters for alerts, cards, card-style tabs, buttons, badges,
  and modal shells.
- Added explicit Preline recipe metadata to UI preview examples so the renderer
  maps A2UI fields into named component variants instead of guessing from ids.
- Replaced the prototype `capsem-ai` provider implementation with a
  Capsem-owned HTTP provider while keeping the clean text/image/embedding
  request/result surface at the public boundary.
