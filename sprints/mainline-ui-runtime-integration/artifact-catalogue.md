# Rich Artifact Catalogue

## Principle

Everything visible is editable and user-configurable, so everything visible is
backed by a workspace/Loro document node with schema, provenance, renderer
coverage, topology, and mutation rules. The renderer backend can vary; the
contract cannot.

## Artifact Families

| Family | First artifact types | Renderer backend | Required topology |
| --- | --- | --- | --- |
| UI components | card, alert, ask, table, facts, notice, tool result | Loro state projected to Svelte/Preline and A2UI Basic where applicable | title, body, actions, table rows/cells, task controls, embedded media |
| Charts | barChart, lineChart, heatmap, boxPlot, scatterPlot | Plotly | title, legend, x axis, y axis, secondary y axis, series, data point/row where practical |
| Diagrams | mermaidDiagram | Mermaid first | title, diagram body, nodes/edges if extractable |
| Timelines | timeline | Pure Preline timeline template rendered by Svelte/Web Component | title, event, lane, date, annotation |
| Spreadsheets | spreadsheet, sheet | Workbook model with Svelte grid preview/editor and XLS/XLSX/Google-compatible export semantics | workbook, sheet tab, cell, range, formula, chart embed |
| Slides | slideDeck, slide | Deck model with Svelte preview/editor and PPTX/Google Slides-compatible export semantics | deck title, slide, region, text block, image, chart, diagram |
| Media | image, video, audio | generated media or external URI renderer | title, media body, caption, source/provenance, controls |
| Websites | website, page, form | Svelte/Preline preview first; browser preview later | page, section, form, field, label, validation, action |

## Chart API Shape

The first chart surface should be explicit, not a loose Plotly JSON escape hatch:

- `ui.chart.barChart(title, series, xLabel, yLabel, xUnit, yUnit, direction, stack, legend)`
- `ui.chart.lineChart(title, series, xLabel, yLabel, xUnit, yUnit, legend)`
- `ui.chart.heatmap(title, x, y, z, xLabel, yLabel, colorLabel)`
- `ui.chart.boxPlot(title, series, xLabel, yLabel)`
- `ui.chart.scatterPlot(title, series, xLabel, yLabel, fit)`

Required options:

- `direction`: `vertical` or `horizontal`.
- `stack`: `none`, `stacked`, or `grouped`.
- `legend`: `none`, `top`, `right`, or `bottom`.
- `series`: named multi-series data.
- `axis`: units and labels are part of the contract, not renderer decoration.
- `fit`: optional trend/fit metadata, including method and display flag.
- `export`: PNG and SVG first. PDF remains a later decision.

Raw Plotly JSON may be useful as an escape hatch only after validation, but it
must not be the default authoring API for models or plugins.

`PLOTLY_CHART_API_COVERAGE` freezes this API surface in Rust. The current
prototype renderer has initial Plotly rendering for bar/line/scatter/heatmap/box
trace types and stack mode mapping; full fit-line and double-axis visual
coverage still needs dedicated renderer tests.

Frontend source-level drift tests now assert the renderer names every frozen
Plotly branch, including line/scatter, heatmap, box plot, horizontal bars,
stack mode, and SVG/PNG export. Browser visual coverage for every chart family
is still pending.

The browser E2E now creates bar, heatmap, box plot, and scatter artifacts
through the Rust API and waits for each Plotly Shadow DOM renderer to hydrate.
It also asserts generated linear-fit traces and second-y-axis Plotly
trace/layout state for charts that declare `fit` or `secondAxis`.

## Slide And Spreadsheet Direction

Slide decks are composed artifacts:

- A `slideDeck` owns ordered `slide` ids.
- A `slide` owns positioned regions.
- Regions may embed text, image, chart, diagram, table, or generated media.
- Every region has topology so a user can comment on and mutate it.
- The canonical model must be rich enough for PowerPoint/PPTX and Google Slides
  compatibility. The Svelte/Preline renderer is the local preview/editor shell,
  not the source of truth.
- Export/import compatibility is part of the artifact contract, so slide size,
  theme, layout regions, media references, embedded charts, speaker notes, and
  stable ids cannot be renderer-only metadata.
- JavaScript owns the interactive authoring experience: slide ordering,
  selection, region handles, inline text edits, comments, previews, and export
  affordances. Those interactions still emit typed runtime records; JS must not
  become a second document model.
- MCP/local tools must be able to create and update the same deck one slide at a
  time, then export a user-downloadable PowerPoint-compatible artifact.

Spreadsheets are also composed artifacts:

- A `spreadsheet` owns ordered `sheet` ids.
- A `sheet` owns cells/ranges.
- Charts, diagrams, and images can embed into sheets.
- The canonical model must preserve workbook/sheet/cell identity, formulas,
  formats, ranges, chart anchors, and export semantics needed for XLS/XLSX and
  Google Sheets compatibility. The Svelte grid is only the local preview/editor
  surface.
- JavaScript owns the spreadsheet UX: grid navigation, selection, inline edits,
  formula display, filter/sort affordances, chart placement, and export
  controls. Those UI operations must lower to the same typed records/tools.
- MCP/local tools must be able to create a workbook, insert/update data, embed
  or reference charts, and export a user-downloadable spreadsheet artifact.

## Export And Toolchain Direction

Capsem can ship document/rendering tools in its image. The default export path
is therefore VM toolchain first, not Rust-crate-first:

- LibreOffice/OpenOffice or compatible office tooling for PPTX/XLSX/PDF
  conversion when it gives the best compatibility.
- Python libraries such as `python-pptx`, `openpyxl`, and `xlsxwriter` when
  they give the best manipulation/export surface.
- Chromium print, Pandoc, or similar packaged tools when they are the right
  renderer/converter for office/PDF output.
- Plotly and Mermaid are live UI dependencies, not VM-assist document
  converters. The Svelte/Web Component renderer path is authoritative for chart
  and diagram display, and PNG/SVG export must reuse that path or a
  parity-checked headless browser path.
- Rust exporters only when a Rust implementation is clearly the best option for
  quality, security, maintainability, and compatibility.

The runtime contract is:

- Rust validates and persists the canonical artifact model, provenance, export
  requests, output references, timings, and failure reasons.
- JavaScript/Svelte provides the interactive editing and preview UX over that
  same model.
- MCP/local tools are the only agent-facing control plane for creation,
  mutation, inspection, and export.
- Packaged converters/renderers may perform the heavy lifting for PPTX, XLSX,
  PDF, PNG, SVG, and similar formats, as long as their inputs/outputs are typed,
  auditable, and tested.

Required first export surfaces:

- `local.export.slideDeck`: produces a PowerPoint-compatible deck artifact.
- `local.export.spreadsheet`: produces an XLS/XLSX-compatible workbook artifact.
- `local.export.chart`: produces PNG and SVG from the authoritative Plotly UI
  renderer path or a parity-checked headless browser renderer.
- `local.export.diagram`: produces SVG and PNG from the authoritative Mermaid
  UI renderer path or a parity-checked headless browser renderer.
- `local.export.pdf`: produces a PDF for deck/report output, with the exact
  implementation to be selected by spike.

The acceptance test is practical: an agent dictates a slide deck one slide at a
time through MCP/local tools, sees it update in the chat UI, edits individual
regions through topology, exports the deck, and obtains a file the user can
download/upload.

The spreadsheet test mirrors that path: an agent creates a workbook, writes data,
adds a chart, edits a cell/range through topology, exports the workbook, and
obtains a compatible file artifact.

The PDF story is unresolved and must be spiked explicitly. It should be
delegated to Chromium print, LibreOffice/OpenOffice, Pandoc, or another packaged
office/PDF toolchain unless a better option is proven. It must end as a typed
export job with telemetry and a file artifact.

Website/page/form artifacts are composed like slides:

- A `website` owns ordered `page` ids.
- A `page` owns sections and blocks.
- A `form` owns fields, validation copy, actions, and submission metadata.
- Form fields should map to A2UI Basic interactive primitives where possible:
  `TextField`, `CheckBox`, `ChoicePicker`, `Slider`, and `DateTimeInput`.
- The first version must support exact comments and mutations on fields,
  labels, validation text, and actions.

## Diagram And Timeline Direction

Mermaid is the first diagram renderer because it gives us a compact textual
source, broad diagram coverage, and easy model authoring. Production must still
validate the diagram type/source and sandbox rendering.

Timeline now starts as a structured lane/event artifact rather than raw Mermaid,
but the renderer must use the Preline timeline component markup as the template
source. No bespoke timeline approximation should pass the renderer drift gate.
The prototype has the first callable Rust API, schema, generated TS/Python
bindings, `capsem-timeline` renderer, and browser hydration proof; exact
Preline-template parity, filtering, exact-event topology polish, and export
remain follow-up catalogue work.

`MERMAID_DIAGRAM_API_COVERAGE` freezes Mermaid as a strict-sandbox source
renderer with SVG/PNG export expectations. `TIMELINE_API_COVERAGE` freezes
timeline as a structured lane/event artifact rather than raw diagram text, and
`local.ui.timeline` is now explicit in the prototype descriptor surface.

## Generated Media

Generated image/video/audio are artifacts, not blobs floating outside the
runtime:

- Prompt and provider metadata are provenance.
- Cost/tokens/provider timing come from `capsem-ai`.
- The persisted artifact stores a content reference, not necessarily inline
  bytes.
- UI cards are Loro-backed component nodes and render media through the same
  topology and mutation rules.

`GENERATED_MEDIA_API_COVERAGE` freezes the generation/provenance path. Text,
image, and embedding are explicit in the prototype; video and audio are named
but deferred until provider support is wired.

`validate_native_artifact()` is now the first runtime wall for emitted native
artifacts. It validates generated media provenance/usage placeholders,
table/sheet/deck required fields, supported Plotly chart kinds and option
compatibility, Mermaid diagram specs, and slide/deck block structure before
constructors return artifacts.

`native_artifact_schema()` emits the first checked JSON Schema snapshot at
`schemas/capsem-ui/artifacts/native-artifact.v1.schema.json`. The Rust test
keeps the schema isomorphic with the code contract; TS/Python bindings should
be generated from this snapshot next rather than hand-maintained.

The current TypeScript and Python helpers now enforce the same contract shape
with discriminated artifact/spec parsing. The enum/component/spec bindings are
generated from the schema snapshot by `npm run generate:artifact-types`, and
`npm run check:artifact-types` fails when the checked-in generated files drift.

## Conformance Rule

The machine-readable source for the artifact and tool boundary is
`crates/capsem-ui-catalog/src/contract_matrix.rs`.

`RICH_ARTIFACT_SCHEMA_COVERAGE` now freezes the day-one generic artifact schema
families:

- slideDeck, slide
- spreadsheet, sheet
- website, page, form
- image, video, audio
- chart, diagram, timeline

For every artifact type we ship:

- Rust schema/type exists.
- TS generated type or parser exists.
- Renderer exists.
- Topology extractor exists.
- Mutation allowlist exists.
- Snapshot/replay test exists.
- Browser visual test exists for light/dark theme when applicable.
