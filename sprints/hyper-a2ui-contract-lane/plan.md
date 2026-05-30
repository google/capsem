# Hyper A2UI Contract Lane Sprint

## Goal

Build a pure Rust HTTP lane on Hyper that emits A2UI contract output without
depending on Axum, Svelte, or the browser workbench. The lane must prove:

```text
typed UI tool request -> Rust contract lowering -> A2UI JSON -> conformance check
```

Then add a small TypeScript client/decoder library with tests so the frontend
can consume the same wire output without hand-waving.

## Why

The current demo compiles and renders a narrow happy path, but it allowed
semantic contract drift:

- `ui.alert(tone="warning")` accepted `tone` and dropped it in rendering.
- `ui.card(...)` had no link label/href contract, so `visit homepage` could not
  be expressed.

This sprint makes the contract lane explicit. Accepted fields must survive in
the Rust response and become visible to TypeScript.

## Scope

- Add a Hyper-only crate/server module, separate from Axum.
- Support `POST /a2ui/render` with a `UiToolProgram`.
- Return A2UI messages, renderer recipe metadata, and conformance status.
- Add E2E Rust tests that start the Hyper server on TCP and call it over HTTP.
- Validate returned A2UI with `capsem_plugin_engine::ui::validate_messages`.
- Assert `alert.tone` and `card.link` survive into recipe metadata.
- Add a TypeScript client/decoder library and Vitest coverage.

## Files

- `Cargo.toml`
- `crates/capsem-a2ui-hyper/Cargo.toml`
- `crates/capsem-a2ui-hyper/src/lib.rs`
- `crates/capsem-a2ui-hyper/tests/a2ui_http.rs`
- `ui-preview/src/a2uiClient.ts`
- `test/a2ui-client.test.ts`
- `CHANGELOG.md`
- `sprints/hyper-a2ui-contract-lane/tracker.md`

## Done Means

- Hyper server compiles without Axum.
- Rust E2E test posts alert/card programs and validates A2UI output.
- E2E test fails if accepted alert/card fields are dropped.
- TS client parses the same response shape and rejects malformed responses.
- Existing Rust and frontend tests remain green.

## Proof Matrix

- Unit/contract: TS parser tests; Rust conformance helper tests.
- Functional: Hyper E2E POST over TCP.
- Adversarial: TS rejects malformed response; Hyper rejects bad method/path/body.
- E2E/browser: not required for this lane; it proves protocol, not rendering.
- Telemetry/performance: deferred.
