# Capsem Native MCP And Deck Proof Sprint

## Goal

Design and prove the Capsem-native tool surface that little Codex will use to
build rich artifacts: structured data, charts, diagrams, generated media,
assets, previews, exports, slides, and slide decks.

The proof target is a nice slide deck produced through Capsem APIs, not through
ad hoc browser scripting:

```text
SQLite data -> table/sheet -> chart/diagram/generated media -> slide blocks
  -> slide deck -> export artifact -> web preview
```

## Direction

Keep the authoring API elegant and object-shaped, while aligning the MCP layer
with real Capsem routing.

Logical hierarchy for little Codex:

```text
local
  data
    sqlite
      create
      replaceTable
      query
      toSheet
  ui
    alert
    ask
    card
    table
    barChart
    lineChart
    heatmapChart
    boxPlot
    diagram
    sheet
    slide
    slideDeck
  generate
    image
    video
    audio
  asset
    put
    get
    list
  export
    image
    svg
    pdf
    slide
    deck
  web
    preview
      show
      snapshot
      inspect
```

The exact MCP wire naming remains a spike question. Current Capsem guest MCP
uses `server__tool_name` through the aggregator, and host-control tools use
`capsem_*`. Preferred model-facing shape is `local.ui.alert()`. The spike must
test what MCP clients actually accept, then choose one of:

- dotted logical names if robust: `local__ui.alert`
- flattened names if required: `local__ui_alert`
- a display/documentation layer only if clients cannot expose dotted calls

Do not invent a separate Capsem naming scheme without checking the real MCP
client behavior.

## Gemini Generate Spike

The first generation spike uses Gemini because Capsem already has a Gemini API
key path.

- If the Gemini key is configured, `generate.image`, `generate.video`, and
  `generate.audio` call Gemini through the Rust service.
- If the key is missing, the tools return an explicit configuration error.
- Provider routing is later work: Gemini, OpenAI, local, and other providers
  need policy, UI configuration, provenance, cost/latency display, and failure
  state.
- Generation APIs must accept multimodal inputs through typed asset handles or
  typed inline payloads: reference images, masks, screenshots, clips, voice
  samples, existing audio, timing hints, and storyboards.
- Generated output becomes a typed asset handle before it enters UI or export.

## Rust Server First, MCP Second

Before wiring MCP, create a small Python client library that calls the Rust
webserver for every planned API. This is the rehearsal surface.

Python client target:

```python
client.data.sqlite.create(...)
client.data.sqlite.replace_table(...)
client.data.sqlite.query(...)
client.ui.bar_chart(...)
client.generate.image(...)
client.asset.put(...)
client.export.deck(...)
client.web.preview.show(...)
```

The Python library should be tiny and boring:

- no business logic
- typed-ish request helpers
- clear error surfacing
- one method per Rust API route
- acceptance tests that build the deck proof entirely through the client

When the Python-to-Rust path is clean, MCP tools should wrap the same Rust
routes. That ensures the MCP surface is not invented in isolation.

## Web Component Security

`<capsem-elt>` and specialized elements are UI isolation, not a security
boundary.

Required constraints:

- The renderer receives validated typed specs, never arbitrary HTML or JS.
- Specs are cloned/serialized at the boundary; renderers cannot mutate trusted
  host objects.
- Shadow DOM provides style/lifecycle isolation only.
- Component events are allowlisted `capsem:*` events with typed payloads.
- No ambient DOM authority for plugin/model-authored components.
- Custom components resolve through sandboxed resolvers that return catalog
  nodes, not DOM.
- Renderer failures produce typed error UI and telemetry, not host crashes.
- Size limits, teardown hooks, and lifecycle cleanup are part of the contract.
- Generated assets are referenced through asset handles, not path strings or
  raw blobs.

## Plotly Security

Plotly should be a trusted first-party renderer adapter, not a plugin escape
hatch.

Required constraints:

- Plugins and models emit typed Capsem chart specs, not raw Plotly JSON.
- Rust validates chart kind, series, axes, units, stacking, orientation,
  legends, second axis, fit functions, and export intent.
- The Svelte/Web Component adapter lowers the validated spec to Plotly.
- Raw `hovertemplate`, HTML labels, custom JS, external image URLs, and unknown
  Plotly config fields are rejected or sanitized.
- Data size, series count, point count, and export dimensions are bounded.
- Plotly modebar/edit affordances are disabled unless explicitly allowed.
- Export output is deterministic and content-addressed through the asset store.
- Plotly rendering happens in the `<capsem-elt>` family, with the option to
  move heavy or risky rendering into a sandboxed frame if needed.
- Chart rendering and export failures are logged and surfaced as typed errors.

## Done

- Sprint notes agree on the tool hierarchy and MCP naming question.
- Rust webserver routes exist for the first proof path.
- Python client can drive the proof path end to end.
- Gemini-backed `generate.image` works when configured and fails clearly when
  not configured.
- SQLite data can feed table/sheet/chart specs.
- Plotly-backed chart preview and export are validated through typed specs.
- Diagram preview/export works through the diagram API, with Mermaid as the
  first backend.
- Slide and slideDeck specs compose text, table, chart, diagram, image, and
  generated media blocks.
- `web.preview` can show the produced artifact to the user.
- The final acceptance test builds a nice deck without hand-editing the output.

## Proof Matrix

- Unit/contract: Rust spec validation, Python client request serialization,
  chart/diagram/media/deck schema checks.
- Functional: Python client builds the deck from Rust routes.
- Adversarial: malformed specs, missing Gemini key, oversized chart data,
  unsafe Plotly fields, invalid asset handles, invalid slide references.
- E2E/UI: preview the generated deck through `web.preview`.
- Telemetry: each native tool call records duration, result, provider/model
  provenance where relevant, and error class.
- Performance: basic timing for chart render/export, deck export, and Gemini
  calls; no premature throughput theater.
