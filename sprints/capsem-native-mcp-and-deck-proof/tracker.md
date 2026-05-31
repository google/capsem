# Sprint: Capsem Native MCP And Deck Proof

## Tasks

- [x] Write sprint plan and tracker.
- [ ] Decide MCP naming after testing real client behavior.
- [x] Define Rust webserver routes for native Capsem APIs.
- [x] Add tiny Python client library that calls the Rust routes.
- [x] Expose contract calls for `generate.image`, sheet, table, chart,
  diagram, slide, and slideDeck artifact assembly.
- [x] Persist native artifact tool calls in a live workspace for step-by-step
  `/deck` preview.
- [x] Add SQLite data workspace proof.
- [ ] Add Plotly chart spec/render/export proof.
- [ ] Add Mermaid diagram spec/render/export proof.
- [ ] Add Gemini-backed `generate.image` proof.
- [x] Add generated asset store handles.
- [x] Add individual artifact materialization for generated image, sheet,
  table, chart, diagram, slide, and deck.
- [x] Add slide and slideDeck composition proof.
- [x] Add `<capsem-slide-deck>` proof surface.
- [x] Add acceptance test that builds a nice deck end to end.
- [x] Changelog.
- [x] Commit.

## Notes

- Direction: authoring should feel like `local.ui.alert()` and
  `local.generate.image()`, but the implementation must respect real Capsem MCP
  routing and client constraints.
- Current Capsem guest MCP exposes the built-in server as `local` through
  `server__tool_name` names. Host-control MCP uses `capsem_*`.
- Do not duplicate generic tools. GitHub, fetch, Python, JS, and shell-like
  capabilities come from the sandbox or third-party MCPs.
- Capsem-native tools are product objects: UI, data, generated media, assets,
  exports, and previews.
- Python client is a deliberate rehearsal layer: Python -> Rust route now, MCP
  tool -> same Rust route later.
- First implementation slice added `/native/deck-proof`, `/native/artifacts`,
  `/native/artifacts/:id`, `/native/data/sqlite/query`, and
  `/native/ui/render-artifact`.
- The proof currently builds an in-memory SQLite `quarterly_metrics` table,
  materializes individual artifacts, and includes two chart specs from the same
  data workspace.
- Artifact handles are deterministic BLAKE3 `capsem://artifact/{hash}` handles.
- Live server smoke on `127.0.0.1:8791` returned 11 artifacts, 2 charts, 4
  slides, and a `capsem-chart` render artifact response.
- Gemini is the first `.generate` provider only because the key already exists
  in Capsem. Provider configuration is later.
- Web Components and Shadow DOM are not security boundaries.
- Plotly is a first-party renderer adapter. Models/plugins never receive raw
  Plotly authority.
- `web.preview` is for websites/browser-backed page acceptance, not deck
  preview.
- Deck and artifact previews use Capsem-owned `<capsem-*>` components.
- The sprint must prove gradual output: each useful artifact can be produced and
  inspected independently before deck composition.
- Deck is orchestration, not a deck engine: generation, sheets/tables, charts,
  diagrams, slides, and slideDeck are separate typed calls. `slideDeck` only
  assembles slide refs and export intent.
- The nice demo deck is now "The Realms Of Code": five code houses from
  SQLite, an overview table with mottos and armory, two charts, two Mermaid
  diagram specs, one planned Gemini image per house, and one slide per house.
- Browser verification caught a real Web Component registration bug: one class
  constructor cannot be registered for multiple custom element tag names. The
  fix registers per-tag subclasses on the shared `CapsemArtifactElement`
  foundation.
- The live workspace is intentionally in-memory for the spike. `POST
  /native/workspace/reset` clears it, construction routes insert artifacts, and
  `/native/deck-proof` falls back to the Realms demo only when the workspace is
  empty.

## Coverage Ledger

- Unit/contract: `cargo test -p capsem-ui-catalog --test native_deck` passes;
  it now asserts the Realms deck has five house slides, five house image
  artifacts, two charts from the same data sheet, a primitive-call slideDeck
  composition path, and read-only SQLite. `cargo test` passed across the
  workspace before this UI slice; focused server compile checks pass for this
  slice.
- Functional: `PYTHONPATH=python python3 -m unittest discover -s python/tests`
  passes; live server smoke passed for deck proof, SQLite query, and render
  artifact routes. Latest live smoke returned 22 artifacts, 9 slides, 5 house
  slides, 5 house images, 2 charts, and 5 SQLite rows. A second smoke drove
  the route sequence SQLite query -> sheet -> generate image spec -> chart ->
  slide -> slideDeck and got typed `<capsem-*>` artifacts at each step.
- Stepwise UI acceptance: after `reset_workspace()` and only Python client/tool
  calls, `/native/deck-proof` returned `Live Tool Deck` with 5 artifacts and 1
  slide; Chrome `/deck?v=workspace` rendered that same live deck with one
  `<capsem-slide-deck>`.
- Adversarial: Rust test rejects non-SELECT SQL in the demo workspace.
- E2E/UI: Chrome `/deck?v=fix` rendered "The Realms Of Code", registered
  `<capsem-slide-deck>`, showed 22 artifact controls, and mounted the custom
  deck component. Chrome `/deck?v=workspace` rendered the tool-built live
  workspace deck without code changes.
- Telemetry: pending.
- Performance: pending.
- Missing/deferred: real Plotly rendering/export, Mermaid rendering/export,
  Gemini provider call, provider routing UI, final MCP naming, sandboxed frame
  renderer fallback, full deck export formats, website-oriented `web.preview`,
  and plugin/WASM binding.
