# Sprint Plan: Mainline UI Runtime Integration

## What We Are Building

We are turning the isolated prototype into a production-ready Capsem UI runtime. The runtime lets agents, tools, plugins, and users create and mutate rich UI through one record/projection engine instead of ad hoc DOM writes or private routes.

The first mainline feature is the chat/UI authoring lane:

- A generated or tool-created component appears in the UI through a Rust workspace record.
- A user can comment on an exact UI element.
- The comment becomes a persistent task visible to the user.
- An agent can inspect the UI topology and emit a typed mutation.
- The UI updates through projection/replay.
- The user can resolve the task and keep an audit trail.

## Why

Capsem needs one execution model for:

- User-facing chat and side panels.
- Model-generated UI.
- Plugin-generated UI.
- Security review surfaces.
- Charts, tables, slides, diagrams, websites/forms, generated media, and rich artifacts.

If each area gets its own UI protocol, we will pay the complexity cost forever. The record/projection spine gives us replay, audit, toolability, comments, and testing in one place.

## Scope

In scope:

- Rust workspace record model.
- Projection engine and topology model.
- Persistent comments/tasks.
- Typed mutations for title/text/style and component props.
- Svelte/Preline renderer shell.
- Annotation overlay and comment task list.
- Reconnect/replay/checkpoint behavior.
- Local tool/MCP surface for UI inspection and mutation.
- Mainline browser E2E tests.
- Telemetry/audit events for each mutation.

Out of scope:

- Full WASM plugin runtime.
- Plugin marketplace/install flow.
- Polished artifact editors beyond the day-one protocol.
- Production-grade export polish beyond the MCP acceptance gate.
- Full chart/spreadsheet rendering polish beyond the components needed for integration proof.
- Production multi-user collaboration.

Important non-scope distinction: production multi-user collaboration is out of
scope, but a T0A protocol bake-off is in scope. We must test protocol
alternatives before building the production runtime.

Also: polished editors can iterate, but export protocol support and the MCP
acceptance gate are in scope from day one. The contracts must support slides, slide decks,
spreadsheets, sheets, websites, pages, forms, generated image/video/audio,
Plotly charts, Mermaid diagrams, and timelines.

## Architecture Decisions

- Rust is canonical for records, projection, persistence, validation, replay, and mutation acceptance.
- Svelte only renders projections and posts actions.
- Comments are records, not local component state.
- A resolved comment remains visible in the task list until explicitly pruned by checkpoint/retention policy.
- Spatial markers are derived UI affordances, not the durable task.
- Component topology ids are the long-term identity contract. CSS selectors are diagnostic/fallback metadata.
- Style patches are allowed only through a whitelist and should be replaced by typed component-property mutations where possible.
- The local tool surface should be stable enough that Codex can drive it during browser E2E and later through MCP.
- A2UI Basic is the baseline wire format. Capsem extensions are allowed only as
  named catalog components with schema, Rust validation, renderer coverage, and
  topology ids.
- Loro is the editable state model for every user-visible configurable surface:
  cards, alerts, tables, generated media cards, slides, sheets, websites/forms,
  timelines, and richer artifacts. A2UI/Capsem specs are render projections
  from that state, not a separate editable state path.
- Protocol adoption is not assumed. Capsem record protocol, A2UI-native stream,
  A2UI plus Capsem envelope, CRDT document protocol, and MCP event log must be
  compared before implementation. Yrs/Yjs, Automerge, and Loro are CRDT
  candidates inside that protocol bake-off.
- Rich artifacts are first-class workspace records, not side-channel files.
  Renderers may use Plotly, Mermaid, Svelte/Preline, or generated media, but the
  record/projection/topology contract remains the same.

## Files To Create Or Modify

Prototype source files to port:

- `crates/capsem-ui-catalog/src/workspace.rs`
- `crates/capsem-ui-catalog/tests/workspace.rs`
- `ui-preview/src/ChatPage.svelte`
- `ui-preview/src/elements/capsem-elt.ts`
- `ui-preview/src/elements/capsem-artifacts.ts`
- `ui-preview/src/nativeDeck.ts`
- `test/native-workspace.test.ts`
- `test/capsem-element.test.ts`
- `schemas/a2ui/v0_9/**`
- `schemas/capsem-ui/**`

Expected mainline targets, to confirm in T0:

- Rust runtime crate under `crates/`.
- Main UI Svelte component package/module.
- Local service routes or MCP host integration.
- Session DB / telemetry schema migrations.
- Browser E2E tests in the mainline frontend test suite.
- Changelog and developer docs.

## Work Breakdown

### T0 Contract Freeze

- Inventory mainline UI, service, MCP, settings, and telemetry paths.
- Freeze JSON schema or equivalent Rust/TS source of truth for:
  - `WorkspaceRecord`
  - `WorkspaceContent`
  - `RenderProjection`
  - `RenderDelta`
  - `RenderTopology`
  - `UiTask`
  - `AnnotationTarget`
  - `Mutation`
  - `ComponentSpec`
- Decide which prototype fields are temporary and which are production.
- Define component topology id rules.

### T0A Protocol Bake-Off

- Test competing protocol shapes before building:
  - Capsem record protocol.
  - A2UI-native stream.
  - A2UI plus Capsem envelope.
  - CRDT document protocol with Yrs/Yjs, Automerge, or Loro.
  - MCP event log.
- Encode the same fixture in each candidate:
  - chat surface
  - Loro-backed card with image/link/actions
  - Plotly-style chart artifact
  - Mermaid diagram artifact
  - spreadsheet/sheet artifact
  - slide deck/slide artifact
  - website/page/form artifact
  - generated media artifact with provenance/cost
  - comment on topology target
  - typed mutation
  - resolve
  - checkpoint
  - restart replay
- Choose the canonical protocol and state substrate for mainline.
- Build an A2UI coverage matrix:
  - A2UI Basic primitive
  - Rust type/schema coverage
  - high-level `ui.*` helper coverage
  - Svelte/Preline renderer coverage
  - topology coverage
  - mutation coverage
- Freeze the first MCP/tool set and mark deferred tools explicitly.
- Freeze the artifact taxonomy for:
  - card
  - alert/notice/ask
  - slide deck
  - slide
  - spreadsheet
  - sheet
  - website
  - page
  - form
  - image
  - video
  - audio
  - chart
  - diagram
  - timeline
- Add "helper without renderer fails" and "renderer without schema fails" tests.
- Produce `selected-protocol.md` before T1 starts.

### T1 Crate Extraction

- Start only after T0A selects the protocol.
- Extract projection/runtime into a standalone crate.
- Split prototype server-only code from reusable runtime code.
- Add reducer tests for create/replace/delete/select/comment/resolve/mutate/checkpoint.
- Add adversarial tests for invalid targets, stale seqs, malformed mutations, unsupported styles, missing components, and selector misses.

### T2 Persistence

- Add DB/session-backed workspace log.
- Persist checkpoints, comments/tasks, artifact references, mutation records, and projections.
- Restore state on restart.
- Add compaction/pruning policy.
- Ensure resolved comments remain visible in task history while spatial pins can disappear.

### T3 Main UI Port

- Port `capsem-elt` foundation and artifact renderers to the main UI.
- Replace prototype chat shell with production Capsem shell conventions.
- Implement annotation overlay, comment popover, task ledger, theme/dark mode behavior, and reconnect state.
- Keep UI dense and operational, not demo/marketing styled.
- Ensure Preline token usage matches mainline design system.

### T4 Local Tool/MCP Surface

- Expose tools:
  - `local.ui.info`
  - `local.ui.render`
  - `local.ui.comment`
  - `local.ui.resolve`
  - `local.ui.mutate`
  - `local.workspace.snapshot`
  - `local.workspace.stream`
  - `local.workspace.checkpoint`
  - `local.generate.text`
  - `local.generate.image`
  - `local.generate.audio`
  - `local.generate.video`
  - `local.generate.embedding`
  - `local.data.sqlite`
  - `local.data.spreadsheet`
  - `local.data.sheet`
  - `local.export.spreadsheet`
  - `local.export.slideDeck`
  - `local.export.chart`
  - `local.export.diagram`
  - `local.export.pdf`
  - `local.website.render`
  - `local.website.form`
  - `local.diagram.render`
  - `local.web.preview`
- Add a small Python helper library for test-driving the same surface.
- Ensure tool output is structured and small enough for agent use.

### T5 Component Catalogue

- Harden the first production components:
  - card
  - alert
  - ask/comment task
  - table with search/filter/pagination
  - media/image card
  - chart shell
- Add topology ids for user-addressable subparts.
- Add renderer conformance tests for every topology id.
- Track missing A2UI Basic primitives as explicit Pack 02 debt:
  - modal
  - tabs
  - text field
  - checkbox
  - choice picker
  - slider
  - date/time input
  - list
  - video
  - audio player
  - divider
  - icon variants

### T5A Rich Artifact Catalogue

- Define artifact specs and renderers for:
  - `slideDeck`
  - `slide`
  - `spreadsheet`
  - `sheet`
  - `website`
  - `page`
  - `form`
  - `image`
  - `video`
  - `audio`
  - `chart`
  - `diagram`
  - `timeline`
- Chart API must cover the first Plotly-backed science/finance set:
  - bar chart, vertical and horizontal
  - grouped and stacked bars
  - line chart
  - multi-series chart
  - double-axis chart
  - heatmap
  - box plot
  - scatter plot
  - fit/trend function metadata
  - export as PNG and SVG through the authoritative Plotly UI renderer path
- Diagram API uses Mermaid first unless T0A selects another renderer.
- Diagram export must use the authoritative Mermaid UI renderer path or a
  parity-checked headless browser path.
- Timeline can start as a structured artifact rendered through Svelte/Preline;
  later it may lower to Mermaid, Plotly, or a custom renderer depending on
  interaction needs.
- Every rich artifact must expose topology ids for title, legend, axes, series,
  cells, slide regions, website sections, form fields, validation text, media
  captions, and editable text where applicable.

### T6 Telemetry And Security

- Emit telemetry for:
  - record append
  - projection apply
  - comment create/resolve
  - mutation accept/reject/apply
  - tool caller/principal
  - render errors
- Add mutation allowlist and deny unsupported operations loudly.
- Add audit payload with before/after where safe and useful.

### T7 Release Gates

- Full Rust unit/integration tests.
- Frontend typecheck/build.
- Browser E2E through mainline UI.
- Restart persistence proof.
- Dark/theme visual proof.
- Local tool proof.
- Session telemetry proof.
- Changelog and porting notes.

## Done Looks Like

- Mainline app exposes the workspace UI runtime behind a feature flag if needed.
- The T0A decision record is accepted: Capsem append log, Yrs/Yjs, Automerge,
  Loro, A2UI envelope, or another tested protocol shape is chosen deliberately
  for the first integration.
- A user can create a generated image/card in the main UI.
- A user can comment on an exact visible sub-element.
- The comment appears in a visible task list and survives restart.
- A tool/agent can inspect topology, apply a typed mutation, and the UI updates through replay.
- The user can resolve the task; the marker turns into a checkmark and disappears while the resolved task remains visible.
- A2UI translation is proven from high-level typed call to valid wire object to
  explicit Svelte/Preline renderer to stable topology.
- Loro-backed editable state is proven for cards, not only large artifacts.
- The artifact taxonomy is frozen enough that slide deck, spreadsheet,
  website/form, chart, diagram, timeline, and generated media work can land
  without changing the workspace/runtime spine.
- Telemetry shows every significant record and mutation with principal and status.
- The system is documented well enough for another agent to continue plugin/WASM integration on top.

## Testing Matrix

- Unit/contract:
  - Rust reducer and projection tests.
  - Schema round-trip tests.
  - TS parser/render input tests.
  - A2UI coverage matrix tests for every shipped helper/component.
  - Artifact schema round-trip tests for every shipped rich artifact.
  - CRDT candidate spike tests if a CRDT substrate is selected.
- Functional:
  - Mainline routes/tools append records and return frames.
  - Comments and mutations update projection.
  - Checkpoints compact correctly.
- Adversarial:
  - Invalid target id.
  - Unsupported mutation type.
  - Bad selector/topology id.
  - Stale seq/replay gap.
  - Disallowed style/property.
  - Render component missing.
- E2E/browser:
  - Create component.
  - Comment exact element.
  - Reopen/edit comment.
  - Resolve comment.
  - Agent/tool mutation changes the correct element.
  - Reconnect/replay after dropped socket.
  - Restart persistence.
  - Dark mode/theme check.
- Telemetry:
  - Session DB or telemetry stream contains record append, task lifecycle, mutation result, and principal.
- Performance:
  - Projection apply cost for 100/1,000 records.
  - UI render cost for task list and component map.
  - WebSocket reconnect/replay latency.

## Integration Risks

- Mainline UI shell may not match prototype assumptions about layout or tokens.
- Session DB persistence may require migration discipline.
- Stable topology ids require component-specific work; numeric DOM-node ids cannot be the production contract.
- Preline token classes must align with mainline theme configuration.
- Agent-visible tool responses can become too large if `ui.info` returns raw component specs.
- Telemetry can accidentally capture sensitive prompt/artifact data unless fields are classified.
- Pulling in a CRDT library without product need can obscure the audit trail and
  make mutation causality harder to explain.
- Under-covering A2UI Basic will make models generate valid-looking components
  that Capsem cannot render or mutate.
- Shipping MCP tools before the hierarchy is frozen will create tool-name debt.
- Plotly and Mermaid need sandboxing and input validation; generated SVG/HTML
  cannot become an injection lane.
- Slide/spreadsheet objects can become large quickly, so checkpoints and
  `local.ui.info` must return compact summaries by default.

## Release Strategy

- Land runtime crate and tests first.
- Land main UI behind a feature flag.
- Land tool/MCP surface after projection persistence is stable.
- Keep plugin/WASM work unblocked but not coupled to this merge.
- Promote to default only after browser E2E and restart persistence pass.
