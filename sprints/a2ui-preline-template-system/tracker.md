# Sprint: A2UI Preline Template System

## Tasks

- [x] Plan sprint and proof matrix.
- [ ] Vendor/pin A2UI v0.9 schemas.
- [ ] Add Python Preline snippet scraper.
- [ ] Scrape initial Preline snippets into ignored `private/todo/preline`.
- [ ] Define Capsem A2UI catalog schema slice.
- [ ] Add local UI MCP-style authoring tool contract.
- [ ] Define template metadata schema.
- [ ] Promote first reviewed Preline template.
- [ ] Add template checker and tests.
- [ ] Add UI tool lowering tests.
- [ ] Wire Rust preview endpoint to include validation/check reports.
- [ ] Rebuild Svelte workbench with Preline sidebar and inspector panes.
- [ ] Pass self-use acceptance test: agent builds requested UI through UI tools.
- [ ] Promote initial five templates.
- [ ] Browser verification.
- [ ] Changelog.
- [ ] Commit.

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

## Coverage Ledger

- Unit/contract: pending UI tool lowering and template checker tests.
- Functional: pending workbench plus UI tool-created surface.
- Adversarial: pending.
- E2E/VM: pending browser acceptance scenario where the agent builds requested
  UI through the tool path.
- Telemetry: deferred.
- Performance: deferred.
- Missing/deferred: production Capsem gateway integration, plugin telemetry,
  complete A2UI Basic catalog coverage, and final generator selection.
