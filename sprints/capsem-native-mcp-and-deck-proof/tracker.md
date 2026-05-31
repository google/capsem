# Sprint: Capsem Native MCP And Deck Proof

## Tasks

- [x] Write sprint plan and tracker.
- [ ] Decide MCP naming after testing real client behavior.
- [x] Define Rust webserver routes for native Capsem APIs.
- [x] Add tiny Python client library that calls the Rust routes.
- [x] Add SQLite data workspace proof.
- [ ] Add Plotly chart spec/render/export proof.
- [ ] Add Mermaid diagram spec/render/export proof.
- [ ] Add Gemini-backed `generate.image` proof.
- [x] Add generated asset store handles.
- [x] Add individual artifact materialization for generated image, sheet,
  table, chart, diagram, slide, and deck.
- [x] Add slide and slideDeck composition proof.
- [ ] Add `<capsem-slide-deck>` proof surface.
- [ ] Add acceptance test that builds a nice deck end to end.
- [x] Changelog.
- [ ] Commit.

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

## Coverage Ledger

- Unit/contract: `cargo test -p capsem-ui-catalog --test native_deck` passes;
  `cargo test` passes across the workspace.
- Functional: `PYTHONPATH=python python3 -m unittest discover -s python/tests`
  passes; live server smoke passed for deck proof, SQLite query, and render
  artifact routes.
- Adversarial: Rust test rejects non-SELECT SQL in the demo workspace.
- E2E/UI: pending.
- Telemetry: pending.
- Performance: pending.
- Missing/deferred: real Plotly rendering/export, Mermaid rendering/export,
  Gemini provider call, provider routing UI, final MCP naming, sandboxed frame
  renderer fallback, full deck export formats, website-oriented `web.preview`,
  and plugin/WASM binding.
