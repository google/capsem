# Sprint: Generation Provider Lane

## Tasks

- [x] Plan self-contained generation lane
- [x] Add reusable Rust `capsem-ai` crate
- [x] Wire server text/image routes through generation crate
- [x] Wire Python rehearsal client text method
- [x] Add tests
- [x] Changelog
- [ ] Commit

## Notes

- Real Capsem writes detected Gemini credentials as `google-api-key` in
  service settings. The prototype must understand that shape.
- Keep UI artifacts out of the reusable generation crate.
- Renamed the crate from `capsem-generation` to `capsem-ai`; the latter is the
  right abstraction boundary for dynamic review, media, embeddings, and future
  model use.
- Missing `service.toml` is allowed for local prototype use, but malformed
  settings now fail startup instead of silently losing credentials.

## Coverage Ledger

- Unit/contract: `cargo test -p capsem-ai`, `cargo test -p capsem-plugin-server`
- Functional: `cargo test`, Python client route tests, `npm run ui:build`
- Adversarial: missing credential path returns `configMissing` artifact state
- E2E/VM: deferred until main Capsem integration
- Telemetry: existing native telemetry captures `generate.text` and
  `generate.image` status from artifact specs
- Performance: not claimed; first Siumai compile is heavy and should be
  tracked as dependency cost, not runtime proof
