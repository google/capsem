# Sprint: A2UI Preline Template System

## Tasks

- [x] Plan sprint and proof matrix.
- [x] Vendor/pin A2UI v0.9 schemas.
- [x] Add Python Preline snippet scraper.
- [x] Scrape initial Preline snippets into ignored `private/todo/preline`.
- [x] Define Capsem A2UI catalog schema slice.
- [x] Add local UI MCP-style authoring tool contract.
- [x] Define template metadata schema.
- [x] Promote first reviewed Preline template.
- [x] Add template checker and tests.
- [x] Add UI tool lowering tests.
- [x] Wire Rust preview endpoint to include validation/check reports.
- [x] Rebuild Svelte workbench with Preline sidebar and inspector panes.
- [x] Pass self-use acceptance test: agent builds requested UI through UI tools.
- [x] Promote initial five templates.
- [x] Browser verification.
- [x] Changelog.
- [x] Commit.

## Notes

- Direction correction: A2UI is the emitted wire object. Capsem extends A2UI by
  catalog, not by inventing another UI protocol.
- Preline is a checked renderer template layer. Exact Preline snippets are
  scraped into ignored private files first, then reviewed/promoted into tracked
  templates with Capsem binding markers.
- JSON Schema is the source of truth for generated Rust and TypeScript types.
  If generation is not landed in this sprint, that remains explicit coverage
  debt.
- Svelte is the only renderer implementation language in this prototype. It
  receives validated messages plus trusted template/check metadata.
- Acceptance criterion: Codex must be able to use the local UI MCP-style tools
  to build a user-requested surface, validate it, and show it in the workbench.
  Manual JSON editing does not count.
- ArrowJS contributes authoring ergonomics and template/binding vocabulary, but
  raw Arrow/HTML output is not accepted from plugins or models.
- Vendored upstream A2UI v0.9 `server_to_client`, `common_types`, and Basic
  catalog schemas from the Google A2UI repository.
- The Preline scraper found exact snippets for alert discovery, primary
  button, and simple card. Modal basic and top-border card were not found by
  the first bounded extractor, so the promoted templates are reviewed local
  recipes and the scraper manifest records the misses instead of inventing
  source.
- The local UI tool runner now supports catalog describe/list, surface
  create/clear/validate/preview, raw component add, and a convenience
  `ui.alert` lowering path. It rejects raw renderer inputs such as `html`,
  `innerHTML`, `class`, `className`, `style`, and `script`.
- Browser verification caught and fixed a Svelte rune bug: the item list must
  use `$derived.by`, otherwise the workbench loads payload status but never
  advances from the empty item list.

## Coverage Ledger

- Unit/contract:
  - `cargo test -p capsem-plugin-engine -- --nocapture`
  - `cargo test -p capsem-plugin-server -- --nocapture`
  - `npm test -- --run`
- Functional:
  - `npm run ui:build`
  - `curl -fsS http://127.0.0.1:8787/ui/spec/demo` showed
    `toolAcceptance.ok == true`, one tool-built surface, and five examples.
  - The Svelte workbench renders the tool-built surface first and exposes the
    A2UI/tool/template inspectors.
- Adversarial:
  - `ui_tools_reject_raw_renderer_inputs` covers raw renderer input rejection.
  - Template tests reject unknown props and missing/invalid bindings in the
    first checker slice.
- E2E/VM:
  - Chrome DevTools loaded `http://127.0.0.1:8787/?v=ui-tool-gate-2` and
    verified the visible self-use surface, sidebar entries, rendered preview,
    and A2UI inspector text. No Capsem VM integration in this isolated sprint.
- Telemetry: deferred.
- Performance: deferred.
- Missing/deferred: production Capsem gateway integration, plugin telemetry,
  JSON Schema to Rust/TypeScript code generation, full A2UI Basic catalog
  fixture coverage, full Preline snippet extraction, and real template-driven
  Svelte component generation.
