# Sprint: Capsem Native MCP And Deck Proof

## Tasks

- [x] Write sprint plan and tracker.
- [ ] Decide MCP naming after testing real client behavior.
- [ ] Define Rust webserver routes for native Capsem APIs.
- [ ] Add tiny Python client library that calls the Rust routes.
- [ ] Add SQLite data workspace proof.
- [ ] Add Plotly chart spec/render/export proof.
- [ ] Add Mermaid diagram spec/render/export proof.
- [ ] Add Gemini-backed `generate.image` proof.
- [ ] Add generated asset store handles.
- [ ] Add slide and slideDeck composition proof.
- [ ] Add `web.preview` proof surface.
- [ ] Add acceptance test that builds a nice deck end to end.
- [ ] Changelog.
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
- Gemini is the first `.generate` provider only because the key already exists
  in Capsem. Provider configuration is later.
- Web Components and Shadow DOM are not security boundaries.
- Plotly is a first-party renderer adapter. Models/plugins never receive raw
  Plotly authority.

## Coverage Ledger

- Unit/contract: pending.
- Functional: pending.
- Adversarial: pending.
- E2E/UI: pending.
- Telemetry: pending.
- Performance: pending.
- Missing/deferred: provider routing UI, final MCP naming, sandboxed frame
  renderer fallback, full deck export formats, and plugin/WASM binding.
